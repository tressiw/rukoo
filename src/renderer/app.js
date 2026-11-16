import { t, setLanguage, localize, shortcutLabel } from './i18n.js';
import { icons } from './icons.js';
import {
  api,
  esc,
  $,
  $$,
  listTime,
  groupLabel,
  longDate,
  readerDate,
  hhmm,
  numericDate,
  fileSize,
  person,
  formatAddress,
  avatar,
  hue,
  initials,
  cleanPreview,
  toast,
  dropToast,
  showingToast,
  runToastAction,
  showMenu,
  closeMenu,
  menuOpen,
  confirmDialog,
  choiceDialog,
  applyTheme,
  onSystemThemeChange,
  isLight,
  segmented,
  setSegmented,
  afterDrag
} from './ui.js';
import { fillFrame } from './mailframe.js';
import { mountComposer } from './composer.js';
import { openSettings, promptDialog } from './settings.js';
import { openSetup } from './setup.js';
import { openWhatsNew } from './whats-new.js';
import { actionFor, keyLabel, tip, setKeymap } from './keymap.js';
import { initSortActivity, setActivity, setCategories, sortStatusHtml, toggleSortPanel, sortPanelOpen } from './sort-activity.js';
import { planServerSearch as planSearch } from './server-search.js';
import * as calendar from './calendar.js';
import * as meeting from './meeting.js';
import { mountInvite, openInviteFile } from './invite.js';
// The availability panel registers itself with the calendar; applyBooking puts the booking page on the calendar.
import { applyBooking } from './calendar-free.js';
import { splitNewMail, addToToast, newMailToastFor } from './new-mail.js';

// Sidebar entries. `drop` marks a folder role that accepts dragged messages.
const VIEWS = [
  { id: 'inbox', get label() { return t('mailbox.folders.inbox'); }, icon: 'inbox', count: 'strong', drop: 'inbox' },
  { id: 'vip', get label() { return t('mailbox.folders.vip'); }, icon: 'vip', count: 'muted' },
  { id: 'starred', get label() { return t('mailbox.folders.starred'); }, icon: 'star' },
  { id: 'drafts', get label() { return t('mailbox.folders.drafts'); }, icon: 'drafts', count: 'muted' },
  { id: 'sent', get label() { return t('mailbox.folders.sent'); }, icon: 'sent' },
  { id: 'archive', get label() { return t('mailbox.folders.archive'); }, icon: 'archive', role: true, drop: 'archive' },
  { id: 'junk', get label() { return t('mailbox.folders.junk'); }, icon: 'junk', role: true, count: 'muted', drop: 'junk' },
  { id: 'trash', get label() { return t('mailbox.folders.trash'); }, icon: 'trash', drop: 'trash' },
  { id: 'saved', get label() { return t('mailbox.folders.saved'); }, icon: 'saved' }
];
const LABELS = Object.fromEntries(VIEWS.map((v) => [v.id, () => v.label]));
const FILTERS = [
  { id: 'all', get label() { return t('mailbox.filters.all'); } },
  { id: 'unread', get label() { return t('mailbox.filters.unread'); } },
  { id: 'starred', get label() { return t('mailbox.filters.starred'); } },
  { id: 'attachments', get label() { return t('mailbox.filters.attachments'); } }
];
const LIST_MIN = 300;
const LIST_MAX = 640;
const SIDEBAR_W = 236;
const RAIL_W = 60;
// The chat panel docks beside the reader in wide windows and slides over it in narrower ones.
const AGENT_MIN = 320;
const AGENT_MAX = 560;
// The breakpoints in styles.css, asked the way the CSS asks them. At a fractional zoom the window can be 1239.4 CSS px
// wide: the CSS docks the panel there, while window.innerWidth, a whole number, says 1239 (#47).
const OVERLAY_QUERY = matchMedia('(max-width: 1239px)');
const NARROW_QUERY = matchMedia('(max-width: 860px)');

// The window's width in whole CSS px, rounded down, for the sums that leave the reader its 420 px. window.innerWidth
// can round up (1252.8 px gives 1253), which would take a fraction of a pixel from the reader. A media query sees the
// exact width and, unlike measuring an element, costs no layout while the window is being resized.
function viewWidth() {
  const w = window.innerWidth;
  return matchMedia(`(min-width: ${w}px)`).matches ? w : w - 1;
}

export const S = {
  data: null,
  scope: 'all',
  view: 'inbox',
  folder: null,
  query: '',
  searchScope: 'view',
  filter: 'all',
  list: [],
  counts: null,
  selectedId: null,
  // The message last opened while unread: the Unread filter keeps showing it until another opens, also after
  // Back in a narrow window.
  keptId: null,
  checked: new Set(),
  anchorId: null,
  // The keyboard position while Shift+arrows grow a selection. selectedId stays the message on screen.
  cursorId: null,
  message: null,
  loading: false,
  expanded: false,
  details: false,
  attachmentsOpen: true,
  // Stacks of mail from one sender that the user unfolded.
  openStacks: new Set(),
  // The related emails under the open email, { id, list }, from main's related(); and the email whose related emails
  // all show, past the first few.
  related: null,
  relatedAll: null,
  // Open the newest message after the next load (at start and on a folder switch).
  autoOpen: true,
  // The inline editor in the reading pane, when one is open.
  composer: null,
  // Inbox categories from main (categories/), and the tab on screen: a category id, or null for all of the inbox.
  categories: null,
  category: null,
  // Mail older than the cache keeps. olderMore: how many older emails Load older can bring into the list on screen;
  // loadingOlder: the list it is loading for (olderKey()). server: the server search under a search's results,
  // { key, query, state: 'searching', 'done' or 'failed', more, error }; null while nothing is searched.
  olderMore: 0,
  loadingOlder: null,
  server: null
};

// Layout preferences live in the renderer; they only matter to this window. mode: 'mail', 'calendar' or 'meeting'.
const prefs = { listWidth: 400, sidebarCollapsed: false, agentOpen: false, agentWidth: 380, sidebarWithAgent: false, mode: 'mail', relatedOpen: true, ...readPrefs() };
const MODES = ['mail', 'calendar', 'meeting'];
if (!MODES.includes(prefs.mode)) prefs.mode = 'mail';
const calendarMode = () => prefs.mode === 'calendar';
const meetingMode = () => prefs.mode === 'meeting';
function readPrefs() {
  try {
    return JSON.parse(localStorage.getItem('rukoo.layout') || '{}');
  } catch (_) {
    return {};
  }
}
function savePrefs() {
  localStorage.setItem('rukoo.layout', JSON.stringify(prefs));
}

const root = document.documentElement;

// ---------- data ----------

let renderedLanguage = null;
let refreshTimer = null;
let pending = null;
let again = false;

export function scheduleRefresh(ms = 60) {
  clearTimeout(refreshTimer);
  refreshTimer = setTimeout(refresh, ms);
}

// Coalesces overlapping refreshes: a call during a running refresh triggers exactly one more pass.
export function refresh() {
  if (pending) {
    again = true;
    return pending;
  }
  pending = (async () => {
    do {
      again = false;
      await refreshOnce();
    } while (again);
  })().finally(() => {
    pending = null;
  });
  return pending;
}

function searchingAll() {
  return Boolean(S.query.trim()) && S.searchScope !== 'view';
}

// The .ics files main has opened and this window has yet to show (inviteFiles in main.js). Only once the first state
// is in, so the card is in the user's language.
async function takeInviteFiles() {
  if (!S.data) return;
  for (const token of await api('inviteFiles').catch(() => [])) openInviteFile(token, { openEvent: openCalendarEvent });
}

// Open in calendar on an invitation's card.
function openCalendarEvent(id, start) {
  setMode('calendar');
  calendar.openEvent(id, Number.isFinite(start) ? start : undefined);
}

async function refreshOnce() {
  const data = await api('state');
  const first = !S.data;
  S.data = data;
  // A default account changed in Settings or a compose window: the editor's link beside From follows it.
  S.composer?.accountsChanged(data);
  const languageChanged = renderedLanguage !== data.settings.language;
  setLanguage(data.settings.language);
  renderedLanguage = data.settings.language;
  localize();
  if (first) takeInviteFiles();
  // In the user's language, now that it is set.
  if (first) takeUndoNotices();
  if (languageChanged) {
    if (S.message) renderReader();
    S.composer?.retheme(data);
    agentPanel?.relocalize();
    calendar.relocalize();
    meeting.relocalize();
  }
  // With one account, "Alle accounts" would just repeat it; show the account itself.
  if (data.accounts.length === 1) S.scope = data.accounts[0].id;
  else if (!data.accounts.some((a) => a.id === S.scope)) S.scope = 'all';
  if (S.searchScope === 'account' && S.scope === 'all') S.searchScope = 'all';
  const query = searchingAll()
    ? { scope: S.searchScope === 'all' ? 'all' : S.scope, view: 'everything', query: S.query }
    : { scope: S.scope, view: S.view, folder: S.folder, query: S.query };
  const [list, counts, categories, older] = await Promise.all([
    api('list', query),
    api('counts', S.scope),
    api('categoriesState').catch(() => S.categories),
    api('olderState', query).catch(() => ({ more: 0, server: 0 }))
  ]);
  S.list = list;
  S.counts = counts;
  S.categories = categories;
  S.olderMore = older.more;
  planServerSearch(query, older.server);
  setCategories(categories);
  setKeymap(data.settings.keymap);
  if (S.updates === undefined) S.updates = await api('updatesState').catch(() => null);
  const ids = new Set(list.map((m) => m.id));
  for (const id of S.checked) if (!ids.has(id)) S.checked.delete(id);
  const wasLight = isLight();
  applyTheme(data.settings.theme);
  // A theme picked in Settings lands here, after main's 'theme' event redrew with the old one:
  // redraw the mail and the editor's quote in the theme the page now has.
  if (isLight() !== wasLight && !languageChanged) {
    if (S.message) renderReader();
    S.composer?.retheme(data);
  }
  renderShell();
  // After the shell, which holds the reading pane; the newest email that opens below makes way for the editor.
  if (first) takeSendFailures();
  if (!data.accounts.length) {
    if (!document.querySelector('.page.setup')) openSetup(ctx, { first: true });
    return;
  }
  renderSearch();
  renderSidebar();
  renderList();
  reconcileReader();
  if (S.checked.size > 1) renderReader();
  else renderReaderNav();
  autoOpen();
  // "What's new" opens once for everyone, on the first start of a version that has one.
  if (!S.whatsNewAsked) {
    S.whatsNewAsked = true;
    api('whatsNewState')
      .then((w) => w && w.show && showWhatsNew())
      .catch(() => {});
  }
  // Account names and colours show in the calendar too.
  if (calendarMode()) {
    if (calendar.isOpen()) calendar.reload();
    else calendar.show();
  }
  if (meetingMode() && !meeting.isOpen()) meeting.show();
}

// ---------- older mail ----------

// The list on screen, as Load older takes it.
function olderKey() {
  return JSON.stringify([S.scope, S.view, S.folder || null]);
}

// Under a search's results Rukoo also searches the server for older mail than it keeps, once the typing stops, when
// the folders searched have any (onServer, from olderState). query is the request the list on screen came from, which
// the search follows (server-search.js). A search with another text, scope or folder starts again; a result for one
// that is no longer on screen is dropped.
let serverTimer = null;
function planServerSearch(query, onServer) {
  const next = planSearch(S.server, query, onServer);
  if (next === S.server) return;
  clearTimeout(serverTimer);
  S.server = next;
  if (next) serverTimer = setTimeout(() => searchServer(next.key), 450);
}

// Main searches a few folders per account at a time (engine.searchServer()), so this asks for the next ones until none
// are left, and the list shows what the folders searched so far found meanwhile. A search no longer on screen stops.
async function searchServer(key) {
  const s = S.server;
  if (!s || s.key !== key) return;
  let more = 0;
  const errors = [];
  for (let resume = false; ; resume = true) {
    let res;
    try {
      res = await api('searchServer', { ...s.query, resume });
    } catch (err) {
      res = { errors: [err.message] };
    }
    if (S.server !== s) return;
    more += res.more || 0;
    errors.push(...(res.errors || []));
    if (!res.remaining) break;
    await refresh().catch(() => {});
    if (S.server !== s) return;
  }
  S.server = { ...s, state: errors.length ? 'failed' : 'done', more, error: errors.join('\n') };
  refresh();
}

function retryServerSearch() {
  if (!S.server) return;
  S.server = { ...S.server, state: 'searching' };
  renderList();
  searchServer(S.server.key);
}

async function loadOlder() {
  if (S.loadingOlder) return;
  const key = olderKey();
  const focused = document.activeElement && document.activeElement.closest('.list-older');
  S.loadingOlder = key;
  renderList();
  try {
    await api('loadOlder', { scope: S.scope, view: S.view, folder: S.folder });
  } catch (err) {
    toast(t('mailbox.older.loadFailed', { error: err.message }), 6000);
  }
  S.loadingOlder = null;
  await refresh();
  // The button is drawn again; the keyboard stays on it, or on the list once everything is in.
  if (focused && olderKey() === key) ($('.list-older .btn') || $('.list-scroll'))?.focus();
}

// The row at the end of the list. Without a search: Load older, while the server has more of the folders listed.
// Under a search: how the search of older mail on the server went.
function olderHtml() {
  if (S.query.trim()) {
    const s = S.server;
    if (!s) return '';
    if (s.state === 'searching') return `<div class="list-older"><span class="spinner"></span><span class="t">${esc(t('mailbox.older.searching'))}</span></div>`;
    if (s.state === 'failed') {
      return `<div class="list-older failed"><span class="t">${esc(t('mailbox.older.failed', { error: s.error }))}</span><button class="btn secondary sm" data-action="older-retry" data-i18n="common.actions.retry">${esc(t('common.actions.retry'))}</button></div>`;
    }
    const n = rows().filter((m) => m.older).length;
    const more = s.more ? `<span class="h">${esc(t('mailbox.older.more', { count: s.more }))}</span>` : '';
    return `<div class="list-older"><span class="t">${esc(n ? t('mailbox.older.found', { count: n }) : t('mailbox.older.none'))}</span>${more}</div>`;
  }
  if (!S.olderMore || S.view === 'saved') return '';
  // While it loads, the spinner takes the icon's place, so the button keeps its width; its name says it is loading.
  const loading = S.loadingOlder === olderKey();
  const busy = loading ? ` disabled aria-busy="true" aria-label="${esc(t('mailbox.older.loading'))}"` : '';
  return `<div class="list-older"><button class="btn secondary sm" data-action="older-load"${busy}><span class="ic">${loading ? '<span class="spinner"></span>' : icons.down}</span>${esc(t('mailbox.older.load'))}</button><span class="h">${esc(t('mailbox.older.available', { count: S.olderMore }))}</span></div>`;
}

// Show the newest message instead of an empty reading pane. It stays unread until you open it yourself.
function autoOpen() {
  if (!S.autoOpen) return;
  S.autoOpen = false;
  if (S.composer || S.selectedId || S.checked.size || S.query.trim() || NARROW_QUERY.matches) return;
  // The newest message, whatever order the list is sorted in; a stack it is folded into opens with it.
  const newest = rows().reduce((a, b) => (!a || b.date > a.date ? b : a), null);
  if (newest) openMessage(newest.id, { auto: true });
}

function account(id) {
  return S.data.accounts.find((a) => a.id === id);
}

function scopedAccounts() {
  return S.scope === 'all' ? S.data.accounts : [account(S.scope)].filter(Boolean);
}

function viewLabel() {
  if (S.view === 'folder') {
    const acc = account(S.scope);
    const f = acc && acc.folders.find((x) => x.path === S.folder);
    return f ? f.name : S.folder;
  }
  return LABELS[S.view]?.() || t('mailbox.folders.inbox');
}

function folderName(m) {
  const acc = account(m.accountId);
  const f = acc && acc.folders.find((x) => x.path === m.folder);
  return f ? f.name : m.folder;
}

// The shortest label that still tells the accounts apart: "gmail", "gmail.com", the name before the @, or the address.
let labelCache = { key: '', labels: {} };
function accountLabel(acc) {
  const accounts = S.data.accounts;
  const key = accounts.map((a) => a.email).join(',');
  if (labelCache.key !== key) {
    const unique = (list) => new Set(list.map((x) => x.toLowerCase())).size === list.length;
    const domain = (a) => a.email.split('@')[1] || a.email;
    const candidates = [
      accounts.map((a) => domain(a).replace(/\.[^.]+$/, '')),
      accounts.map(domain),
      accounts.map((a) => a.email.split('@')[0]),
      accounts.map((a) => a.email)
    ];
    const pick = candidates.find(unique) || candidates[3];
    labelCache = { key, labels: Object.fromEntries(accounts.map((a, i) => [a.id, pick[i]])) };
  }
  return labelCache.labels[acc.id] || acc.email;
}

// Inbox categories show as tabs in the inbox, while categories are on and nothing is searched.
const categoriesOn = () => Boolean(S.categories && S.categories.enabled && S.categories.categories.length) && S.view === 'inbox' && !S.query.trim();
const categoryOf = (id) => (S.categories && S.categories.categories.find((c) => c.id === id)) || null;
// An email's category in a list that mixes them: Inbox itself, a search, or a view built from the inbox such as
// VIPs. Not in a category's own tab, whose header says it already.
const rowCategory = (m) => (m.category && S.categories && S.categories.enabled && !(categoriesOn() && categoryOf(S.category)) ? categoryOf(m.category) : null);


// The categories Inbox itself leaves out: their Show in Inbox switch is off.
const hiddenCategories = () => new Set(((S.categories && S.categories.categories) || []).filter((c) => c.inbox === false).map((c) => c.id));

// The list in the category tab on screen. Inbox itself leaves out the categories set not to show there.
function categoryRows() {
  if (!categoriesOn()) return S.list;
  if (categoryOf(S.category)) return S.list.filter((m) => m.category === S.category);
  const hidden = hiddenCategories();
  return hidden.size ? S.list.filter((m) => !hidden.has(m.category)) : S.list;
}

// The reading pane shows an inbox email only while Inbox or the open category lists it. One that leaves the list,
// because it moved to another category, its category left Inbox or categories came on, leaves the reading pane too.
// The editor stays where it is.
function reconcileReader() {
  if (!S.selectedId || S.composer || !categoriesOn()) return;
  const m = S.list.find((x) => x.id === S.selectedId);
  if (m && !categoryRows().includes(m)) closeReader();
}

// The list as shown: the category tab's list narrowed by the quick filter.
function rows() {
  const list = categoryRows();
  switch (S.filter) {
    case 'unread':
      return list.filter((m) => m.unread || m.id === S.keptId);
    case 'starred':
      return list.filter((m) => m.starred);
    case 'attachments':
      return list.filter((m) => m.hasAttachments);
    default:
      return list;
  }
}

function byId(id) {
  return S.list.find((x) => x.id === id) || (S.message && S.message.id === id ? S.message : null);
}

function checkedMessages() {
  return S.list.filter((m) => S.checked.has(m.id));
}

// What an action applies to: the checked messages, or else the message on screen.
function targets() {
  if (S.checked.size) return checkedMessages();
  // While the editor fills the reading pane, there is no message on screen to act on.
  if (S.composer) return [];
  const m = S.message && (byId(S.message.id) || S.message);
  return m && m.id ? [m] : [];
}

function cursor() {
  return S.cursorId && visibleRows().some((m) => m.id === S.cursorId) ? S.cursorId : S.selectedId;
}

// ---------- shell ----------

function agentDocked() {
  return Boolean(prefs.agentOpen) && !OVERLAY_QUERY.matches;
}

// Docked, the panel gives way so the list keeps its minimum and the reader its 420 px (4 px: the workspace's
// two borders and the two dividers). The chosen width comes back when the window is wider again.
function agentWidth() {
  let max = AGENT_MAX;
  if (agentDocked()) max = Math.min(max, viewWidth() - (sidebarCollapsed() ? RAIL_W : SIDEBAR_W) - LIST_MIN - 420 - 4);
  return Math.round(Math.max(AGENT_MIN, Math.min(max, Number(prefs.agentWidth) || 380)));
}

// With the chat open the folders shrink to the rail, so the screen shows three columns instead of four.
// The rail button can bring them back for as long as the chat is open; below 1440 px a docked chat
// needs the room, so the rail stays.
function sidebarCollapsed() {
  const width = viewWidth();
  if (width < 1100) return true;
  if (prefs.agentOpen) return !prefs.sidebarWithAgent || (agentDocked() && width < 1440);
  return prefs.sidebarCollapsed;
}

function listWidth() {
  // The docked chat plus its handle and the workspace's right edge. The 2 px after it: the workspace's left
  // edge and the divider between the list and the reader.
  const agent = agentDocked() ? agentWidth() + 2 : 0;
  const max = Math.max(LIST_MIN, Math.min(LIST_MAX, viewWidth() - (sidebarCollapsed() ? RAIL_W : SIDEBAR_W) - agent - 2 - 420));
  return Math.round(Math.max(LIST_MIN, Math.min(max, prefs.listWidth)));
}

function renderShell() {
  const app = $('#app');
  if (!app.querySelector('.shell')) {
    app.innerHTML = `<div class="shell">
      <aside class="sidebar" data-i18n-aria-label="mailbox.layout.folders" aria-label="${esc(t('mailbox.layout.folders'))}"><div class="sidebar-mode"></div><div class="sidebar-main"></div></aside>
      <div class="workspace">
        <section class="calendar" data-i18n-aria-label="calendar.title" aria-label="${esc(t('calendar.title'))}"></section>
        <section class="meeting" data-i18n-aria-label="meeting.title" aria-label="${esc(t('meeting.title'))}" data-tab="transcript"></section>
        <section class="listpane" data-i18n-aria-label="mailbox.layout.messages" aria-label="${esc(t('mailbox.layout.messages'))}">
          <div class="list-head"></div>
          <div class="list-tools"></div>
          <button class="new-pill" data-action="new-mail" hidden></button>
          <div class="list-scroll" tabindex="0" role="listbox" aria-multiselectable="true" data-i18n-aria-label="mailbox.layout.messages" aria-label="${esc(t('mailbox.layout.messages'))}"></div>
        </section>
        <div class="divider" role="separator" aria-orientation="vertical" data-i18n-aria-label="mailbox.layout.listWidth" aria-label="${esc(t('mailbox.layout.listWidth'))}" tabindex="0"></div>
        <section class="reader" data-i18n-aria-label="mailbox.layout.message" aria-label="${esc(t('mailbox.layout.message'))}"></section>
      </div>
      <div class="agent-divider" role="separator" aria-orientation="vertical" data-i18n-aria-label="agent.panel.width" aria-label="${esc(t('agent.panel.width'))}" tabindex="0"></div>
      <aside class="agentpane bui" data-i18n-aria-label="agent.panel.label" aria-label="${esc(t('agent.panel.label'))}"></aside>
    </div>`;
    // While Rukoo Voice records a meeting, its clock sits before the chat button, in every mode (meeting.js).
    $('.titlebar-actions').innerHTML = `${meeting.indicatorHtml()}<button class="icon-btn sm" data-agent-toggle data-i18n-title="agent.panel.toggle" title="${esc(t('agent.panel.toggle'))}" aria-pressed="false">${icons.sparkle}</button>`;
    $('.titlebar-actions [data-agent-toggle]').addEventListener('click', () => toggleAgent());
    $('.titlebar-actions [data-meeting-indicator]').addEventListener('click', (e) => openMeeting(e.currentTarget.dataset.meetingIndicator));
    meeting.renderIndicator();
    bindSidebar();
    bindList();
    bindDivider();
    bindAgentDivider();
    bindReader();
    renderReader();
    loadAgentPanel();
  }
  const shell = app.querySelector('.shell');
  const collapsed = sidebarCollapsed();
  shell.classList.toggle('collapsed', collapsed);
  shell.classList.toggle('expanded', S.expanded);
  shell.classList.toggle('has-message', Boolean(S.selectedId) || S.checked.size > 1 || Boolean(S.composer));
  shell.classList.toggle('compact', S.data?.settings.density === 'compact');
  shell.classList.toggle('selecting', S.checked.size > 0);
  shell.classList.toggle('agent-open', Boolean(prefs.agentOpen));
  shell.classList.toggle('calendar-mode', calendarMode());
  document.body.classList.toggle('calendar-mode', calendarMode());
  shell.classList.toggle('meeting-mode', meetingMode());
  document.body.classList.toggle('meeting-mode', meetingMode());
  // The sidebar holds folders, calendars or meetings; its name for screen readers follows, also after a language change.
  const side = shell.querySelector('.sidebar');
  const sideLabel = calendarMode() ? 'calendar.sidebar.calendars' : meetingMode() ? 'meeting.sidebar.label' : 'mailbox.layout.folders';
  side.dataset.i18nAriaLabel = sideLabel;
  side.setAttribute('aria-label', t(sideLabel));
  root.style.setProperty('--sidebar-w', `${collapsed ? RAIL_W : SIDEBAR_W}px`);
  root.style.setProperty('--list-w', `${listWidth()}px`);
  root.style.setProperty('--agent-w', `${agentWidth()}px`);
  for (const b of $$('[data-agent-toggle]')) {
    b.classList.toggle('on', Boolean(prefs.agentOpen));
    b.setAttribute('aria-pressed', String(Boolean(prefs.agentOpen)));
  }
  document.body.classList.toggle('no-accounts', !S.data?.accounts.length);
  document.body.classList.toggle('collapsed-sidebar', collapsed);
}

// ---------- chat panel ----------

// The panel loads on its own, so a fault in it can never keep the mailbox from opening. Main holds the agent
// events it must not miss until the panel says it listens (see agents/gate.js).
let agentPanel = null;
function loadAgentPanel() {
  import('./agent/panel.js')
    .then((m) => {
      agentPanel = m.mountAgentPanel(ctx);
    })
    .catch((err) => console.error('The chat panel did not load:', err));
}

// focus: move the keyboard to the chat input (Ctrl+J and the title bar button do).
function setAgentOpen(open, { focus = false } = {}) {
  const rail = sidebarCollapsed();
  prefs.agentOpen = Boolean(open);
  savePrefs();
  renderShell();
  // The folders become a rail while the chat is open; the rail button's label follows.
  if (sidebarCollapsed() !== rail) renderSidebar();
  if (prefs.agentOpen) agentPanel?.opened({ focus });
  else {
    agentPanel?.closed();
    if ($('.agentpane')?.contains(document.activeElement)) document.activeElement.blur();
  }
}

// The chat panel's news (agent/panel.js renderNews): 'unread' when an answer came in a chat you have not seen, 'waiting'
// when an agent waits for your approval, null for none. The title bar button shows a dot while the panel is closed.
function setAgentNews(tone) {
  for (const b of $$('[data-agent-toggle]')) {
    b.classList.toggle('has-news', Boolean(tone));
    if (tone) b.dataset.news = tone;
    else delete b.dataset.news;
  }
}

function toggleAgent() {
  setAgentOpen(!prefs.agentOpen, { focus: !prefs.agentOpen });
}

// "Add to chat" in the message and bulk menus: the emails go with the message being written in the chat, as when they
// are dragged onto it, and the menus make that a keyboard path too. The chat opens first when it is closed.
function addToChatItem(list) {
  if (!agentPanel) return null;
  return {
    icon: 'sparkle',
    get label() {
      return t('agent.panel.addToChat');
    },
    action: () => {
      if (!prefs.agentOpen) setAgentOpen(true);
      // An open panel follows the email on screen only after agentViewChanged's wait; it follows now, so the emails
      // go to the chat it is about to show, not the one before.
      else agentPanel.sync();
      return agentPanel.addEmails(list.map((m) => m.id));
    }
  };
}

// Main keeps track of what is on screen so agents can see it too; the panel follows the open email.
let viewTimer = null;
function agentViewChanged() {
  clearTimeout(viewTimer);
  viewTimer = setTimeout(() => {
    const c = S.composer;
    api('agentView', {
      mode: meetingMode() ? 'meeting' : calendarMode() ? 'calendar' : 'mail',
      meetingId: meeting.current()?.id || null,
      openMessageId: S.selectedId || null,
      checkedIds: [...S.checked].slice(0, 50),
      scope: S.scope,
      view: S.view,
      folder: S.folder,
      composer: c ? { mode: c.opts.mode, replyToId: c.opts.mode !== 'draft' ? c.opts.id || null : null, draftId: c.draftId || null } : null
    }).catch(() => {});
    agentPanel?.sync();
  }, 150);
}

function bindAgentDivider() {
  const div = $('.agent-divider');
  const set = (w) => {
    prefs.agentWidth = Math.round(w);
    root.style.setProperty('--agent-w', `${agentWidth()}px`);
    root.style.setProperty('--list-w', `${listWidth()}px`);
  };
  div.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    div.setPointerCapture(e.pointerId);
    div.classList.add('dragging');
    const right = $('.shell').getBoundingClientRect().right;
    const move = (ev) => set(right - ev.clientX);
    const up = () => {
      div.classList.remove('dragging');
      div.removeEventListener('pointermove', move);
      div.removeEventListener('pointerup', up);
      prefs.agentWidth = agentWidth();
      savePrefs();
      renderShell();
    };
    div.addEventListener('pointermove', move);
    div.addEventListener('pointerup', up);
  });
  div.addEventListener('dblclick', () => {
    set(380);
    savePrefs();
    renderShell();
  });
  div.addEventListener('keydown', (e) => {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    set(agentWidth() + (e.key === 'ArrowLeft' ? 24 : -24));
    prefs.agentWidth = agentWidth();
    savePrefs();
    renderShell();
  });
}

// ---------- search (in the title bar) ----------

function searchScopeLabel(scope = S.searchScope) {
  if (scope === 'account') return t('mailbox.search.account');
  if (scope === 'all') return S.data.accounts.length > 1 ? t('mailbox.search.allAccounts') : t('mailbox.search.allFolders');
  return t('mailbox.search.folder');
}

function renderSearch() {
  const box = $('.titlebar-search');
  if (!box.querySelector('input')) {
    box.innerHTML = `<span class="s-ic">${icons.search}</span>
      <input id="search" type="search" autocomplete="off" spellcheck="false" data-i18n-aria-label="mailbox.search.label" aria-label="${esc(t('mailbox.search.label'))}"/>
      <button class="search-scope" data-action="search-scope" data-i18n-title="mailbox.search.scope" title="${esc(t('mailbox.search.scope'))}" aria-haspopup="menu"><span></span>${icons.chevronDown}</button>
      <kbd class="search-kbd">${esc(shortcutLabel('Ctrl+E'))}</kbd>`;
    const input = box.querySelector('input');
    input.addEventListener('input', () => {
      S.query = input.value;
      box.classList.toggle('has-text', Boolean(input.value));
      scheduleRefresh(120);
    });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        if (input.value) clearSearch();
        else $('.list-scroll').focus();
      }
      if (e.key === 'ArrowDown' || e.key === 'Enter') {
        e.preventDefault();
        const first = visibleRows()[0];
        if (first) openMessage(first.id);
        $('.list-scroll').focus();
      }
    });
    box.querySelector('.search-scope').addEventListener('click', (e) => {
      const multi = S.data.accounts.length > 1;
      const options = ['view', multi && S.scope !== 'all' ? 'account' : null, 'all'].filter(Boolean);
      showMenu(
        e.currentTarget,
        [
          { heading: t('mailbox.search.in') },
          ...options.map((o) => ({
            label: o === 'view' ? t('mailbox.search.folderNamed', { folder: viewLabel() }) : searchScopeLabel(o),
            checked: S.searchScope === o,
            action: () => {
              S.searchScope = o;
              refresh();
              input.focus();
            }
          }))
        ],
        { align: 'right' }
      );
    });
  }
  const input = box.querySelector('input');
  box.title = `${t('mailbox.search.label')} (${shortcutLabel('Ctrl+E')})`;
  input.placeholder = S.searchScope === 'view' ? t('mailbox.search.placeholder', { scope: viewLabel() }) : t('mailbox.search.placeholder', { scope: searchScopeLabel().toLowerCase() });
  box.querySelector('.search-scope span').textContent = searchScopeLabel();
  if (document.activeElement !== input && input.value !== S.query) input.value = S.query;
  box.classList.toggle('has-text', Boolean(S.query));
}

function focusSearch() {
  const input = $('#search');
  if (!input) return;
  input.focus();
  input.select();
}

function clearSearch() {
  S.query = '';
  const input = $('#search');
  if (input) input.value = '';
  $('.titlebar-search')?.classList.remove('has-text');
  refresh();
}

// ---------- sidebar ----------

// The sidebar's sync line, for the mail accounts in scope or, in the calendar, the calendar accounts.
function syncStatus(accounts = calendarMode() ? calendar.syncAccounts() : scopedAccounts()) {
  if (accounts.some((a) => a.syncing)) return { cls: 'busy', text: t('mailbox.sync.syncing'), title: '' };
  const failed = accounts.find((a) => a.error);
  if (failed) return { cls: 'error', text: t('mailbox.sync.failed'), title: `${failed.email}: ${failed.error}` };
  const times = accounts.map((a) => a.lastSync).filter(Boolean);
  if (!times.length) return { cls: '', text: t('mailbox.sync.never'), title: '' };
  const syncedAt = Math.min(...times);
  const minutes = Math.floor((Date.now() - syncedAt) / 60000);
  const today = new Date(syncedAt).toDateString() === new Date().toDateString();
  const text =
    minutes < 1 ? t('mailbox.sync.justNow') : minutes < 60 ? t('mailbox.sync.minutesAgo', { count: minutes }) : today ? t('mailbox.sync.time', { time: hhmm(syncedAt) }) : t('mailbox.sync.date', { date: numericDate(syncedAt) });
  return { cls: 'ok', text, title: t('mailbox.sync.tooltip', { date: numericDate(syncedAt), time: hhmm(syncedAt) }) };
}

// Keeps "5 min geleden" current without re-rendering the sidebar.
setInterval(() => {
  const el = $('.sync-status');
  if (!el || !S.data) return;
  const status = meetingMode() ? meeting.footStatus() : syncStatus();
  el.className = `sync-status ${status.cls}`;
  el.title = status.title;
  el.querySelector('.t').textContent = status.text;
}, 30000);

function accountAvatar(acc) {
  return `<span class="avatar acc" style="--acc:${esc(acc.color || '#6f9cf2')}" aria-hidden="true">${esc((acc.name || acc.email).trim()[0] || '?').toUpperCase()}</span>`;
}

// Mail, Calendar and Meetings at the top of the sidebar. The switch stays in place, so its thumb slides.
function renderModeSwitch() {
  const box = $('.sidebar-mode');
  const options = [
    { value: 'mail', label: t('calendar.mode.mail'), title: t('calendar.mode.mailShortcut'), icon: 'unread' },
    { value: 'calendar', label: t('calendar.mode.calendar'), title: t('calendar.mode.calendarShortcut'), icon: 'calendar' },
    { value: 'meeting', label: t('meeting.mode.label'), title: t('meeting.mode.shortcut'), icon: 'mic' }
  ];
  const el = box.querySelector('.segmented');
  if (el && el.dataset.language === S.data.settings.language) setSegmented(el, prefs.mode);
  else {
    box.innerHTML = segmented({ name: 'mode', label: t('calendar.mode.label'), value: prefs.mode, options });
    box.querySelector('.segmented').dataset.language = S.data.settings.language;
  }
  // Booking requests that wait for an answer: a count on the Calendar segment, also while the mailbox shows.
  const count = calendar.requestCount();
  const seg = box.querySelector('[data-mode="calendar"]');
  let badge = seg.querySelector('.seg-badge');
  if (!count) {
    badge?.remove();
    seg.removeAttribute('aria-label');
    return;
  }
  if (!badge) {
    badge = document.createElement('span');
    badge.className = 'seg-badge';
    badge.setAttribute('aria-hidden', 'true');
    seg.appendChild(badge);
  }
  badge.textContent = count > 99 ? '99+' : String(count);
  seg.setAttribute('aria-label', t('calendar.mode.calendarRequests', { count }));
}

function setMode(mode) {
  if (mode === prefs.mode || !MODES.includes(mode)) return;
  const was = prefs.mode;
  prefs.mode = mode;
  savePrefs();
  closeMenu();
  renderShell();
  renderSidebar();
  if (was === 'calendar') calendar.hide();
  if (was === 'meeting') meeting.hide();
  if (mode === 'calendar') calendar.show();
  else if (mode === 'meeting') meeting.show();
  else {
    renderList();
    $('.list-scroll')?.focus();
  }
  // The chat panel follows the meeting on screen in meeting mode, and the email everywhere else.
  agentViewChanged();
}

// The meeting view on one meeting: from the title bar's indicator, a calendar card or a new recording.
function openMeeting(id) {
  if (!id || !S.data || !S.data.accounts.length) return;
  setMode('meeting');
  meeting.open(id);
}

// A newer Rukoo on GitHub: a small button above the line over the sync status. Where this install updates itself
// (main's installer.js) it shows once the download is done and restarts into the new version; while it downloads
// there is no button. Elsewhere, and when the download failed, it opens the release page.
function updateButton() {
  const u = S.updates && S.updates.available;
  if (!u) return '';
  const install = S.updates.install && S.updates.install.version === u.version ? S.updates.install : null;
  if (install && install.status === 'ready') {
    return `<button class="update-btn" data-action="update-install" title="${esc(t('updates.button.restartTitle', { version: u.version }))}">${icons.sync}<span>${esc(t('updates.button.restart'))}</span></button>`;
  }
  if (S.updates.install && (!install || install.status !== 'failed') && u.feed) return '';
  return `<button class="update-btn" data-action="update" title="${esc(t('updates.button.title', { version: u.version }))}">${icons.download}<span>${esc(t('updates.button.label', { version: u.version }))}</span></button>`;
}

function sidebarFoot() {
  const status = meetingMode() ? meeting.footStatus() : syncStatus();
  return `${updateButton()}${sortStatusHtml()}<div class="sidebar-foot">
      <button class="sync-status ${status.cls}" data-action="sync" title="${esc(status.title)}"><span class="dot"></span><span class="t">${esc(status.text)}</span></button>
      <button class="icon-btn" data-action="settings" data-i18n-title="settings.title.short" title="${esc(t('settings.title.short'))}">${icons.settings}</button>
      <button class="icon-btn" data-action="collapse" title="${sidebarCollapsed() ? t('mailbox.sidebar.expand') : t('mailbox.sidebar.collapse')}">${icons.panelLeft}</button>
    </div>`;
}

// While an email is dragged, the sidebar isn't drawn again: that would take the folder row under the pointer off the
// page, and the drop with it. A redraw asked for meanwhile happens when the drag is over.
let mailDrag = false;
let sidebarStale = false;

function startMailDrag() {
  mailDrag = true;
  document.body.classList.add('dragging-mail');
  afterDrag(endMailDrag);
}

function endMailDrag() {
  if (!mailDrag) return;
  mailDrag = false;
  document.body.classList.remove('dragging-mail');
  $$('.nav-item.drop-target').forEach((x) => x.classList.remove('drop-target'));
  if (sidebarStale) renderSidebar();
}

function renderSidebar() {
  if (mailDrag) {
    sidebarStale = true;
    return;
  }
  sidebarStale = false;
  renderModeSwitch();
  if (calendarMode()) {
    $('.sidebar-main').innerHTML = calendar.sidebarHtml() + sidebarFoot();
    return;
  }
  if (meetingMode()) {
    $('.sidebar-main').innerHTML = meeting.sidebarHtml() + sidebarFoot();
    return;
  }
  const el = $('.sidebar-main');
  const counts = S.counts;
  const current = account(S.scope);
  const accounts = S.data.accounts;
  const hidden = new Set(S.data.settings.hiddenViews || []);
  const roles = new Set(scopedAccounts().flatMap((a) => a.folders.map((f) => f.role)));
  const fmt = (n) => (n > 999 ? '999+' : String(n));
  const countHtml = (v, n) => {
    if (!v.count || !n) return '';
    return `<span class="count ${v.count}" aria-label="${esc(t(v.id === 'drafts' ? 'mailbox.sidebar.draftCount' : 'mailbox.sidebar.unreadCount', { count: n }))}">${fmt(n)}</span>`;
  };

  const views = VIEWS.filter((v) => !hidden.has(v.id) && (!v.role || roles.has(v.id)));
  // Inbox categories sit under Inbox, each with its unread count while the inbox is open. A category Inbox doesn't
  // show gets a crossed-out eye.
  const subs = S.categories && S.categories.enabled ? S.categories.categories : [];
  const subRows = () =>
    subs
      .map((c) => {
        const on = S.view === 'inbox' && S.category === c.id;
        const n = S.view === 'inbox' ? S.list.filter((m) => m.unread && m.category === c.id).length : 0;
        const off = c.inbox === false;
        const mark = off ? `<span class="cat-off" role="img" aria-label="${esc(t('mailbox.categories.hidden'))}">${icons.eyeOff}</span>` : '';
        return `<button class="nav-item nav-sub ${on ? 'active' : ''}" data-view="inbox" data-category="${esc(c.id)}" style="--cat:${esc(c.color)}" title="${esc(off ? t('mailbox.categories.hiddenTitle', { name: c.name }) : c.name)}"><span class="ic"><span class="cdot"></span></span><span class="label">${esc(c.name)}</span>${mark}${n ? `<span class="count">${n}</span>` : ''}</button>`;
      })
      .join('');
  const viewRows = views
    .map((v) => {
      const active = S.view === v.id && !(v.id === 'inbox' && subs.length && S.category);
      return `<button class="nav-item ${active ? 'active' : ''}" data-view="${v.id}" ${v.drop ? `data-drop-role="${v.drop}"` : ''} title="${esc(v.label)}" ${
        active ? 'aria-current="page"' : ''
      }><span class="ic">${icons[v.icon]}</span><span class="label">${esc(v.label)}</span>${countHtml(v, counts.views[v.id] || 0)}</button>${v.id === 'inbox' ? subRows() : ''}`;
    })
    .join('');

  // Your own folders get a colour of their own, and a letter in the collapsed rail.
  let folderRows = '';
  if (current) {
    const user = current.folders.filter((f) => !f.role && !hidden.has(`folder:${f.path}`));
    folderRows =
      `<div class="nav-section"><span data-i18n="mailbox.layout.folders">${esc(t('mailbox.layout.folders'))}</span><button class="icon-btn xs" data-action="new-folder" data-i18n-title="mailbox.folders.new" title="${esc(t('mailbox.folders.new'))}">${icons.plus}</button></div>` +
      user
        .map((f) => {
          const active = S.view === 'folder' && S.folder === f.path;
          const n = counts.folders[f.path] || 0;
          return `<button class="nav-item ${active ? 'active' : ''}" data-folder="${esc(f.path)}" data-drop-path="${esc(f.path)}" title="${esc(f.name)}" ${
            active ? 'aria-current="page"' : ''
          }><span class="ic fdot" style="--h:${hue(f.name)}"><i></i><b>${esc(initials({ name: f.name }).slice(0, 1))}</b></span><span class="label">${esc(f.name)}</span>${
            n ? `<span class="count muted">${fmt(n)}</span>` : ''
          }</button>`;
        })
        .join('');
  }

  const who = current
    ? `${accountAvatar(current)}<span class="who"><span class="name">${esc(current.name || current.email.split('@')[0])}</span><span class="email">${esc(current.email)}</span></span>`
    : `<span class="avatar acc all" aria-hidden="true">${icons.inbox}</span><span class="who"><span class="name" data-i18n="mailbox.search.allAccounts">${esc(t('mailbox.search.allAccounts'))}</span><span class="email">${esc(t('mailbox.sidebar.accountCount', { count: accounts.length }))}</span></span>`;

  el.innerHTML = `
    <button class="account-switch" data-action="accounts" aria-haspopup="menu" title="${esc(current ? current.email : t('mailbox.search.allAccounts'))}">${who}<span class="chev">${icons.chevronUpDown}</span></button>
    <button class="btn compose-btn" data-action="compose" title="${esc(tip(t('composer.titles.new'), 'compose'))}">${icons.compose}<span data-i18n="composer.titles.new">${esc(t('composer.titles.new'))}</span></button>
    <nav class="nav" data-i18n-aria-label="mailbox.layout.folders" aria-label="${esc(t('mailbox.layout.folders'))}">${viewRows}${folderRows}</nav>
    ${sidebarFoot()}`;
}

function accountMenu(anchor) {
  const accounts = S.data.accounts;
  const counts = S.counts.accounts || {};
  // Each account by the display name set in Settings, with its address underneath; the address alone without a name.
  const items = accounts.map((a) => {
    const name = (a.name || '').trim();
    return {
      icon: accountAvatar(a),
      label: name || a.email,
      sub: name && name.toLowerCase() !== a.email.toLowerCase() ? a.email : '',
      hint: counts[a.id] ? String(counts[a.id]) : '',
      current: S.scope === a.id,
      action: () => switchScope(a.id)
    };
  });
  if (accounts.length > 1) {
    items.push({ icon: 'inbox', get label() { return t('mailbox.search.allAccounts'); }, hint: S.counts.all ? String(S.counts.all) : '', current: S.scope === 'all', action: () => switchScope('all') });
  }
  items.push({ separator: true });
  items.push({ icon: 'plus', get label() { return t('settings.accounts.add'); }, action: () => openSetup(ctx, { first: false }) });
  items.push({ icon: 'settings', get label() { return t('settings.accounts.overview'); }, action: () => openSettings(ctx) });
  showMenu(anchor, items, { align: 'left' });
}

function switchScope(scope) {
  S.scope = scope;
  if (S.view === 'folder') {
    S.view = 'inbox';
    S.folder = null;
  }
  changeView();
}

function bindSidebar() {
  const el = $('.sidebar');
  el.addEventListener('click', (e) => {
    const t = e.target.closest('button');
    if (!t) return;
    if (t.dataset.mode) return setMode(t.dataset.mode);
    if (calendarMode() && calendar.sidebarClick(t)) return;
    if (meetingMode() && meeting.sidebarClick(t)) return;
    switch (t.dataset.action) {
      case 'accounts':
        return accountMenu(t);
      case 'compose':
        return compose({ mode: 'new' });
      case 'settings':
        return openSettings(ctx);
      case 'sync':
        // In meeting mode the line says how Rukoo Voice is; Settings has the details.
        if (meetingMode()) return openSettings(ctx, { view: 'meetings' });
        return calendarMode() ? calendar.syncNow() : syncNow();
      case 'collapse':
        if (prefs.agentOpen) prefs.sidebarWithAgent = sidebarCollapsed();
        else prefs.sidebarCollapsed = !sidebarCollapsed();
        savePrefs();
        renderShell();
        return renderSidebar();
      case 'new-folder':
        return newFolder();
      case 'update':
        return api('updatesOpen');
      case 'update-install':
        return api('updatesInstall');
      case 'sort-activity':
        return toggleSortPanel(t);
    }
    if (t.dataset.view) {
      // Inbox shows all of it, apart from categories it doesn't show; a category under it shows that category.
      if (t.dataset.view === 'inbox') S.category = t.dataset.category || null;
      // A search shows no categories, so a category picked during one ends the search (changeView) first.
      if (t.dataset.category && S.view === 'inbox' && !S.query.trim()) {
        clearChecks();
        S.filter = 'all';
        renderSidebar();
        renderList();
        return reconcileReader();
      }
      S.view = t.dataset.view;
      S.folder = null;
      return changeView();
    }
    if (t.dataset.folder) {
      S.view = 'folder';
      S.folder = t.dataset.folder;
      changeView();
      api('openFolder', S.scope, S.folder).catch((err) => toast(err.message));
    }
  });

  // Messages dragged from the list can be dropped on folders. Taken on dragenter as well as dragover, as in the chat
  // panel (agent/panel.js): Chromium refuses a drop when the last update before it was a dragenter nobody took.
  const target = (e) => e.target.closest('[data-drop-role], [data-drop-path]');
  const accept = (e) => {
    const t = target(e);
    if (!t || !e.dataTransfer.types.includes('application/x-rukoo-ids')) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    $$('.nav-item.drop-target').forEach((x) => x !== t && x.classList.remove('drop-target'));
    t.classList.add('drop-target');
  };
  el.addEventListener('dragenter', accept);
  el.addEventListener('dragover', accept);
  el.addEventListener('dragleave', (e) => {
    const t = target(e);
    if (t && !t.contains(e.relatedTarget)) t.classList.remove('drop-target');
  });
  el.addEventListener('drop', (e) => {
    const t = target(e);
    $$('.nav-item.drop-target').forEach((x) => x.classList.remove('drop-target'));
    if (!t) return;
    e.preventDefault();
    let ids = [];
    try {
      ids = JSON.parse(e.dataTransfer.getData('application/x-rukoo-ids'));
    } catch (_) {
      return;
    }
    const list = ids.map(byId).filter(Boolean);
    const label = t.querySelector('.label')?.textContent || '';
    if (t.dataset.dropRole === 'trash') return removeMessages(list);
    if (t.dataset.dropRole) return moveMessages(list, { role: t.dataset.dropRole }, label);
    moveMessages(list, { path: t.dataset.dropPath, accountId: S.scope }, label);
  });
}

function changeView() {
  S.query = '';
  S.filter = 'all';
  S.keptId = null;
  S.checked.clear();
  newRows.clear();
  S.autoOpen = true;
  // A message from the previous folder would look like it belongs to this one.
  if (S.selectedId) closeReader();
  const input = $('#search');
  if (input) input.value = '';
  $('.list-scroll')?.scrollTo(0, 0);
  refresh();
}

async function newFolder() {
  const name = await promptDialog(t('mailbox.folders.new'), '', { placeholder: t('mailbox.folders.name') });
  if (!name || !name.trim()) return;
  try {
    const path = await api('createFolder', S.scope, name);
    toast(t('mailbox.folders.created', { name: name.trim() }));
    S.view = 'folder';
    S.folder = path;
    changeView();
  } catch (err) {
    toast(err.message, 5000);
  }
}

// ---------- list ----------

function senderLine(m) {
  if (m.role === 'sent' || m.role === 'drafts') {
    const names = (m.to || []).map(person).filter(Boolean);
    return names.length ? t('mailbox.list.sentTo', { recipients: names.join(', ') }) : t('mailbox.message.noRecipient');
  }
  return person(m.from) || t('mailbox.message.unknownSender');
}

// A search that found nothing in the cache while the server is still searched: the list waits for the server's answer.
function waitingForServer() {
  return Boolean(S.query.trim() && S.server && S.server.state === 'searching' && !rows().length);
}

function listTitle() {
  if (S.query.trim()) {
    const query = S.query.trim();
    const scope = S.searchScope === 'view' ? viewLabel() : searchScopeLabel().toLowerCase();
    // Without a count until the server answers, which would otherwise say 0 above "Searching older mail".
    const sub = waitingForServer() ? t('mailbox.search.summaryWaiting', { query, scope }) : t('mailbox.search.summary', { count: rows().length, query, scope });
    return { title: t('mailbox.search.results'), sub };
  }
  const acc = account(S.scope);
  const unread = S.view === 'inbox' || S.view === 'folder' || S.view === 'vip' || S.view === 'junk' || S.view === 'archive' ? categoryRows().filter((m) => m.unread).length : 0;
  const where = acc ? acc.email : t('mailbox.search.allAccounts');
  // A category under Inbox: its name, with the inbox it belongs to.
  const cat = categoriesOn() && categoryOf(S.category);
  if (cat) {
    const n = categoryRows().filter((m) => m.unread).length;
    return { title: cat.name, sub: n ? t('mailbox.list.unreadSummary', { count: n, scope: where }) : where, color: cat.color };
  }
  // The inbox count in the sidebar adds the unread mail older than the messages the list holds; name it apart here,
  // where the list's own unread mail is counted.
  const older = S.view === 'inbox' ? S.counts?.olderUnread || 0 : 0;
  if (older) {
    return {
      title: viewLabel(),
      sub: unread ? t('mailbox.list.unreadSummaryOlder', { count: unread, older, scope: where }) : t('mailbox.list.olderUnreadSummary', { older, scope: where }),
      hint: t('mailbox.list.olderUnreadHint', { count: older, kept: S.counts.inboxLimit })
    };
  }
  return { title: viewLabel(), sub: unread ? t('mailbox.list.unreadSummary', { count: unread, scope: where }) : where };
}

// The HTML each toolbar was last drawn with. A redraw with nothing new, such as the second one after a language change
// or a sync that found no mail, keeps the buttons: the focused one stays focused, and Escape in an open menu can go
// back to the button that opened it.
const drawn = new WeakMap();
function drawHtml(el, html) {
  if (drawn.get(el) === html) return;
  el.innerHTML = html;
  drawn.set(el, html);
}

function renderListHead() {
  const head = $('.list-head');
  const { title, sub, hint, color } = listTitle();
  // A category's tab wears its color, so it can't be taken for Inbox itself.
  head.classList.toggle('cat-head', Boolean(color));
  if (color) head.style.setProperty('--cat', color);
  else head.style.removeProperty('--cat');
  drawHtml(head, `<div class="list-title"><h1>${esc(title)}</h1><div class="sub"${hint ? ` title="${esc(hint)}"` : ''}>${esc(sub)}</div></div>
    ${S.query.trim() ? `<button class="tbtn" data-action="search-clear" data-i18n-title="mailbox.search.clearShortcut" title="${esc(t('mailbox.search.clearShortcut'))}">${icons.close}<span data-i18n="common.actions.clear">${esc(t('common.actions.clear'))}</span></button>` : ''}
    <button class="icon-btn" data-action="sync" title="${esc(tip(t('mailbox.actions.sync'), 'sync'))}">${icons.sync}</button>
    <button class="icon-btn" data-action="list-more" data-i18n-title="common.actions.moreOptions" title="${esc(t('common.actions.moreOptions'))}" aria-haspopup="menu">${icons.more}</button>`);
}

function renderListTools() {
  const tools = $('.list-tools');
  if (S.checked.size) {
    const n = S.checked.size;
    const all = rows().length > 0 && rows().every((m) => S.checked.has(m.id));
    const list = checkedMessages();
    const anyUnread = list.some((m) => m.unread);
    const canArchive = list.some((m) => account(m.accountId)?.archive && m.role !== 'archive');
    tools.className = 'list-tools selecting';
    drawHtml(tools, `<button class="check ${all ? 'on' : 'some'}" data-action="check-all" title="${all ? t('mailbox.selection.none') : tip(t('mailbox.selection.all'), 'selectAll')}" role="checkbox" aria-checked="${all ? 'true' : 'mixed'}">${icons.check}</button>
      <span class="sel-count">${esc(t('mailbox.selection.count', { count: n }))}</span>
      <span class="spacer"></span>
      <button class="icon-btn" data-bulk="${anyUnread ? 'read' : 'unread'}" title="${esc(anyUnread ? tip(t('mailbox.actions.read'), readKey(true)) : tip(t('mailbox.actions.unread'), readKey(false)))}">${anyUnread ? icons.mailOpen : icons.markUnread}</button>
      <button class="icon-btn" data-bulk="star" data-i18n-title="mailbox.actions.star" title="${esc(t('mailbox.actions.star'))}">${icons.star}</button>
      <button class="icon-btn" data-bulk="move" title="${esc(tip(t('common.actions.move'), 'move'))}">${icons.move}</button>
      ${canArchive ? `<button class="icon-btn" data-bulk="archive" title="${esc(tip(t('common.actions.archive'), 'archive'))}">${icons.archive}</button>` : ''}
      <button class="icon-btn" data-bulk="delete" title="${esc(tip(t('common.actions.delete'), 'delete'))}">${icons.trash}</button>
      <button class="icon-btn" data-bulk="more" data-i18n-title="reader.actions.more" title="${esc(t('reader.actions.more'))}" aria-haspopup="menu">${icons.more}</button>
      <button class="icon-btn" data-bulk="clear" data-i18n-title="mailbox.selection.clearShortcut" title="${esc(t('mailbox.selection.clearShortcut'))}">${icons.close}</button>`);
    return;
  }
  tools.className = 'list-tools';
  const list = categoryRows();
  const n = (id) => {
    if (id === 'unread') return list.filter((m) => m.unread).length;
    if (id === 'starred') return list.filter((m) => m.starred).length;
    if (id === 'attachments') return list.filter((m) => m.hasAttachments).length;
    return 0;
  };
  drawHtml(tools, `<div class="filters" role="tablist" data-i18n-aria-label="mailbox.filters.label" aria-label="${esc(t('mailbox.filters.label'))}">${FILTERS.map((f) => {
    const c = n(f.id);
    return `<button class="filter ${S.filter === f.id ? 'active' : ''}" role="tab" aria-selected="${S.filter === f.id}" data-filter="${f.id}">${esc(f.label)}${c ? `<span class="n">${c}</span>` : ''}</button>`;
  }).join('')}</div>`);
}

// Rows folding out of the list (see letGo). Each renders the list once it is gone.
const leaving = new Set();
const LEAVE_MS = 320;

function renderList() {
  // Re-rendering mid-drag would detach the row being swiped; render once the gesture ends.
  if (swipeState.el) {
    swipeState.renderPending = true;
    return;
  }
  // Likewise for a row that is folding away.
  if (leaving.size) return;
  renderShell();
  renderListHead();
  // A selection only holds emails the tab shows; one moved to another category leaves it.
  let pruned = false;
  if (categoriesOn() && S.checked.size) {
    const shown = new Set(categoryRows().map((m) => m.id));
    for (const id of S.checked) if (!shown.has(id)) pruned = S.checked.delete(id) || pruned;
  }
  renderListTools();
  const scroll = $('.list-scroll');
  scroll.innerHTML = listHtml();
  renderShell();
  updateNewPill();
  // The reader showed the selection; it follows what is left of it, as after a move or delete.
  if (pruned) {
    if (S.checked.size > 1 || $('.reader .multi')) renderReader();
    agentViewChanged();
  }
}

// With stacks on in Settings, three or more read messages in a row from one sender in one account, on the same day,
// fold into one row. Unread mail keeps a row of its own, so a stack never hides new mail.
function stackKey(m) {
  if (m.role === 'sent' || m.role === 'drafts' || m.unread) return null;
  const address = ((m.from && m.from.address) || '').toLowerCase();
  return address ? `${m.accountId}|${address}` : null;
}

function stackLayout() {
  const list = rows();
  // heads: a stack's first row, by its id; headOf: the first row of the stack an email is folded into.
  const out = { heads: new Map(), hidden: new Set(), members: new Set(), headOf: new Map() };
  const byDate = S.data.settings.sort === 'date-desc' || S.data.settings.sort === 'date-asc';
  if (!S.data.settings.stackSenders || !byDate || S.query.trim()) return out;
  for (let i = 0; i < list.length; ) {
    const key = stackKey(list[i]);
    const day = groupLabel(list[i].date);
    let j = i + 1;
    if (key) while (j < list.length && stackKey(list[j]) === key && groupLabel(list[j].date) === day) j++;
    const rest = list.slice(i + 1, j);
    if (key && rest.length >= 2) {
      const id = `${day}|${key}`;
      // A selected message inside a stack keeps it open, so the selection never hides.
      const forced = rest.some((m) => m.id === S.selectedId || S.checked.has(m.id));
      const open = forced || S.openStacks.has(id);
      out.heads.set(list[i].id, { id, count: rest.length + 1, open, forced, ids: rest.map((m) => m.id) });
      for (const m of rest) {
        (open ? out.members : out.hidden).add(m.id);
        out.headOf.set(m.id, list[i].id);
      }
    }
    i = j;
  }
  return out;
}

// → opens the stack at the cursor and ← closes it, from its first row or from an email inside it, which then hands
// the cursor to the first row. A row outside a stack leaves the arrows alone. They act in the list, or with nothing
// focused, so they still scroll the open email and resize the dividers. A stack with checked emails inside stays open,
// as it does on screen.
function stackArrow(e) {
  if (e.target !== document.body && !e.target.closest?.('.list-scroll')) return false;
  const at = cursor();
  const layout = stackLayout();
  const headId = layout.heads.has(at) ? at : layout.headOf.get(at);
  const stack = headId && layout.heads.get(headId);
  if (!stack) return false;
  if (e.key === 'ArrowRight') {
    if (stack.open) return false;
    S.openStacks.add(stack.id);
    renderList();
    return true;
  }
  if (!stack.open || stack.ids.some((id) => S.checked.has(id))) return false;
  const fold = () => {
    S.openStacks.delete(stack.id);
    S.cursorId = headId;
    renderList();
    revealRow(headId);
  };
  // The open email keeps its stack open, so the first row opens in its place. With the editor in the reading pane
  // that waits until the editor has let go.
  if (stack.ids.includes(S.selectedId)) {
    const opening = openMessage(headId);
    if (S.selectedId !== headId) {
      opening.then(() => S.selectedId === headId && fold());
      return true;
    }
  }
  fold();
  return true;
}

// The rows on screen, in order: the list without the folded stack members.
function visibleRows() {
  const { hidden } = stackLayout();
  return rows().filter((m) => !hidden.has(m.id));
}

function listHtml() {
  const list = rows();
  if (!list.length) {
    const scoped = scopedAccounts();
    const syncing = scoped.some((a) => a.syncing);
    const failed = scoped.find((a) => a.error);
    let msg = t('mailbox.empty.folder');
    let hint = '';
    // Nothing in the cache yet, while the server is searched: that answer is the one to wait for.
    if (waitingForServer()) {
      return `<div class="empty"><span class="spinner"></span><div class="h">${esc(t('mailbox.older.searching'))}</div></div>`;
    }
    if (S.query.trim()) {
      msg = t('mailbox.empty.search');
      hint = S.searchScope === 'view' ? t('mailbox.empty.searchAll') : t('mailbox.empty.searchOther');
    } else if (S.filter === 'all' && categoriesOn() && categoryOf(S.category)) {
      msg = esc(t('mailbox.categories.empty', { name: categoryOf(S.category).name }));
    } else if (S.filter !== 'all') {
      msg = t('mailbox.empty.filter');
      hint = `<button class="link-btn" data-filter="all" data-i18n="mailbox.empty.showAll">${esc(t('mailbox.empty.showAll'))}</button>`;
    } else if (syncing) msg = t('mailbox.empty.syncing');
    const error = failed && !syncing ? `<p class="error">${esc(failed.email)}: ${esc(failed.error)}</p>` : '';
    const searchAll =
      S.query.trim() && S.searchScope === 'view' ? `<button class="btn secondary sm" data-action="search-everywhere" data-i18n="mailbox.empty.searchAllAction">${esc(t('mailbox.empty.searchAllAction'))}</button>` : '';
    // A failed server search says so; a filter that finds nothing can still load older mail.
    const older = !S.query.trim() || (S.server && S.server.state === 'failed') ? olderHtml() : '';
    return `<div class="empty">${icons.mailOpen}<div class="t">${msg}</div>${hint && !hint.startsWith('<') ? `<div class="h">${hint}</div>` : hint}${searchAll}${error}</div>${older}`;
  }
  const byDate = S.data.settings.sort === 'date-desc' || S.data.settings.sort === 'date-asc';
  const opts = {
    multi: S.data.accounts.length > 1 && (S.scope === 'all' || searchingAll()),
    showFolder: searchingAll() || S.view === 'starred'
  };
  const layout = stackLayout();
  let out = '';
  let label = null;
  for (const m of list) {
    if (layout.hidden.has(m.id)) continue;
    const l = byDate ? groupLabel(m.date) : null;
    if (l !== label) {
      label = l;
      if (l) out += `<div class="group-head" role="presentation">${esc(l)}</div>`;
    }
    out += itemHtml(m, { ...opts, stack: layout.heads.get(m.id), stacked: layout.members.has(m.id) });
  }
  wantLogos(list);
  return out + olderHtml();
}

function itemHtml(m, { multi, showFolder, stack, stacked }) {
  const acc = multi && account(m.accountId);
  const cat = rowCategory(m);
  const draft = m.role === 'drafts';
  const pills = [
    m.vip ? `<span class="pill vip" data-i18n="mailbox.message.vip">${esc(t('mailbox.message.vip'))}</span>` : '',
    showFolder ? `<span class="pill">${esc(folderName(m))}</span>` : '',
    cat ? `<span class="pill cat" style="--cat:${esc(cat.color)}" title="${esc(cat.name)}">${esc(cat.name)}</span>` : ''
  ].join('');
  // With several accounts in one list, the account is a dot in its color on the sender's picture, so it can't be
  // taken for a category's label.
  const accDot = acc
    ? `<span class="acc-dot" style="--acc:${esc(acc.color)}" role="img" title="${esc(acc.email)}" aria-label="${esc(t('mailbox.message.account', { account: accountLabel(acc) }))}"></span>`
    : '';
  const swipe = swipeHtml(m);
  const quickStar = (S.data.settings.quickActions || []).includes('star');
  const selected = S.selectedId === m.id;
  const checked = S.checked.has(m.id);
  const atCursor = (S.cursorId || S.selectedId) === m.id;
  const who = m.role === 'sent' || draft ? (m.to || [])[0] : m.from;
  const preview = cleanPreview(m.preview);
  // A folded stack shows a pile and the count of the emails under it; the arrow keys open and close it (stackArrow).
  const stackLabel = stack && (stack.open ? t('mailbox.message.collapse') : t('mailbox.message.stack', { count: stack.count - 1, sender: person(m.from) }));
  const stackBtn = stack
    ? stack.open
      ? stack.forced
        ? ''
        : `<button class="stack-btn open" data-stack="${esc(stack.id)}" tabindex="-1" title="${esc(`${stackLabel} (←)`)}" aria-label="${esc(stackLabel)}" aria-expanded="true">${icons.up}</button>`
      : `<button class="stack-btn" data-stack="${esc(stack.id)}" tabindex="-1" title="${esc(`${stackLabel} (→)`)}" aria-label="${esc(stackLabel)}" aria-expanded="false">${icons.stack}<span>+${stack.count - 1}</span></button>`
    : '';
  return `<div class="item-wrap ${stacked ? 'stacked' : ''} ${stack && !stack.open ? 'folded' : ''}" data-id="${esc(m.id)}">${swipe}
    <div class="item ${m.unread ? 'unread' : ''} ${selected ? 'selected' : ''} ${checked ? 'checked' : ''} ${atCursor ? 'cursor' : ''} ${stack && !stack.open ? 'folded' : ''}" data-id="${esc(m.id)}" role="option" aria-selected="${selected || checked}" draggable="true">
      <span class="unread-dot" aria-hidden="true"></span>
      <div class="lead">${senderAvatar(who)}${accDot}<button class="check ${checked ? 'on' : ''}" data-check role="checkbox" aria-checked="${checked}" data-i18n-aria-label="mailbox.message.select" aria-label="${esc(t('mailbox.message.select'))}" tabindex="-1">${icons.check}</button></div>
      <div class="body">
        <div class="line1">
          <span class="sender">${draft ? `<span class="draft-tag" data-i18n="composer.titles.draft">${esc(t('composer.titles.draft'))}</span>` : ''}${esc(senderLine(m))}</span>
          ${stackBtn}
          ${quickHtml(m)}<span class="meta">${m.answered ? `<span class="ic-ans" data-i18n-title="mailbox.message.replied" title="${esc(t('mailbox.message.replied'))}">${icons.reply}</span>` : ''}${m.hasAttachments ? `<span class="ic-att" data-i18n-title="mailbox.message.attachment" title="${esc(t('mailbox.message.attachment'))}">${icons.clip}</span>` : ''}<time>${listTime(m.date)}</time></span>
        </div>
        <div class="line2">
          <span class="subject">${esc(m.subject || t('mailbox.message.noSubject'))}</span><span class="sep"> - </span><span class="preview-inline">${esc(preview)}</span>
          <span class="flags">${pills}${
            m.role === 'saved' || (quickStar && !m.starred)
              ? ''
              : `<button class="star-btn ${m.starred ? 'on' : ''}" data-star tabindex="-1" title="${m.starred ? t('mailbox.actions.unstar') : t('mailbox.actions.addStar')}">${m.starred ? icons.starFilled : icons.star}</button>`
          }</span>
        </div>
        <div class="preview">${esc(preview)}</div>
      </div>
    </div>
  </div>`;
}

// ---------- sender logos ----------

// address -> data url, or null when there is none (people, or nothing found).
const logos = new Map();
const logoQueue = new Set();
let logoTimer = null;

function senderAvatar(who, cls = '') {
  const address = ((who && who.address) || '').toLowerCase();
  const logo = address && logos.get(address);
  if (logo) return `<span class="avatar logo ${cls}" data-addr="${esc(address)}" aria-hidden="true"><img src="${logo}" alt=""></span>`;
  return avatar(who || { name: '?' }, cls).replace('<span class="avatar', `<span data-addr="${esc(address)}" class="avatar`);
}

function wantLogos(list) {
  if (S.data.settings.senderLogos === false) return;
  for (const m of list) {
    const who = m.role === 'sent' || m.role === 'drafts' ? (m.to || [])[0] : m.from;
    const address = ((who && who.address) || '').toLowerCase();
    if (address && !logos.has(address)) logoQueue.add(address);
  }
  if (logoQueue.size && !logoTimer) logoTimer = setTimeout(fetchLogos, 30);
}

// One request per sender, so a slow site does not hold up the others; each logo appears when it arrives.
function fetchLogos() {
  logoTimer = null;
  for (const address of logoQueue) {
    logoQueue.delete(address);
    logos.set(address, null);
    api('senderLogos', [address])
      .then((found) => {
        const url = found && found[address];
        if (!url) return;
        logos.set(address, url);
        for (const el of $$(`.avatar[data-addr="${CSS.escape(address)}"]:not(.logo)`)) {
          el.classList.add('logo');
          el.innerHTML = `<img src="${url}" alt="">`;
        }
      })
      .catch(() => {});
  }
}

function syncRowClasses() {
  // A selection inside a folded stack unfolds it.
  if (S.selectedId && !$(`.list-scroll .item[data-id="${CSS.escape(S.selectedId)}"]`) && rows().some((m) => m.id === S.selectedId)) {
    const scroll = $('.list-scroll');
    const top = scroll ? scroll.scrollTop : 0;
    scroll.innerHTML = listHtml();
    scroll.scrollTop = top;
  }
  const at = cursor();
  for (const el of $$('.list-scroll .item')) {
    const id = el.dataset.id;
    const selected = id === S.selectedId;
    const checked = S.checked.has(id);
    el.classList.toggle('selected', selected);
    el.classList.toggle('checked', checked);
    el.classList.toggle('cursor', id === at);
    el.setAttribute('aria-selected', String(selected || checked));
    const box = el.querySelector('.check');
    box?.classList.toggle('on', checked);
    box?.setAttribute('aria-checked', String(checked));
  }
}

function revealRow(id) {
  $(`.list-scroll .item[data-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: 'nearest' });
}

// The message the Unread filter kept leaves once another opens. Its row, and a day heading it leaves empty,
// fold away before the list redraws without them:
//     0ms  the row fades out where it is
//   110ms  its height closes and the rows below move up
//   320ms  the list redraws
function letGo(id) {
  const m = S.list.find((x) => x.id === id);
  if (!m || m.unread || S.filter !== 'unread') return;
  const wrap = $(`.list-scroll .item-wrap[data-id="${CSS.escape(id)}"]`);
  if (!wrap || matchMedia('(prefers-reduced-motion: reduce)').matches) return renderList();
  const els = [wrap];
  const head = wrap.previousElementSibling;
  const next = wrap.nextElementSibling;
  if (head?.classList.contains('group-head') && (!next || next.classList.contains('group-head'))) els.push(head);
  leaving.add(id);
  for (const el of els) {
    const { paddingTop, paddingBottom } = getComputedStyle(el);
    const height = `${el.offsetHeight}px`;
    el.style.overflow = 'hidden';
    el.style.pointerEvents = 'none';
    el.animate(
      [
        { height, paddingTop, paddingBottom, opacity: 1, easing: 'cubic-bezier(0, 0, 0.2, 1)' },
        { height, paddingTop, paddingBottom, opacity: 0, offset: 110 / LEAVE_MS, easing: 'cubic-bezier(0.77, 0, 0.175, 1)' },
        { height: '0px', paddingTop: '0px', paddingBottom: '0px', opacity: 0 }
      ],
      { duration: LEAVE_MS, fill: 'forwards' }
    );
  }
  // A timer, not the animation's promise: it also fires when a redraw took the row away mid-animation.
  setTimeout(() => {
    leaving.delete(id);
    renderList();
  }, LEAVE_MS);
}

function selectionChanged() {
  syncRowClasses();
  renderListTools();
  renderShell();
  // The reader shows the selection while it holds several messages, and the open message otherwise.
  if (S.checked.size > 1 || $('.reader .multi')) renderReader();
  agentViewChanged();
}

function toggleCheck(id) {
  // The first Ctrl+click keeps the open message in the selection, like Explorer and Outlook.
  if (!S.checked.size && S.selectedId && S.selectedId !== id && rows().some((m) => m.id === S.selectedId)) S.checked.add(S.selectedId);
  if (S.checked.has(id)) S.checked.delete(id);
  else S.checked.add(id);
  S.anchorId = id;
  S.cursorId = id;
  selectionChanged();
}

function checkRange(toId) {
  const list = visibleRows();
  const from = list.findIndex((m) => m.id === (S.anchorId || S.selectedId));
  const to = list.findIndex((m) => m.id === toId);
  if (to < 0) return;
  const [a, b] = from < 0 ? [to, to] : [Math.min(from, to), Math.max(from, to)];
  S.checked = new Set(list.slice(a, b + 1).map((m) => m.id));
  S.cursorId = toId;
  selectionChanged();
}

function clearChecks() {
  if (!S.checked.size) return false;
  S.checked.clear();
  S.cursorId = S.selectedId;
  selectionChanged();
  return true;
}

function bindList() {
  const pane = $('.listpane');
  const scroll = $('.list-scroll');
  pane.addEventListener('click', (e) => {
    const btn = e.target.closest('button');
    if (btn && btn.dataset.action) return listAction(btn.dataset.action, btn);
    if (btn && btn.dataset.filter) {
      S.filter = btn.dataset.filter;
      return renderList();
    }
    if (btn && btn.dataset.quick) {
      const m = byId(btn.closest('.item-wrap')?.dataset.id);
      if (m) rowAction(btn.dataset.quick, m);
      return;
    }
    if (btn && btn.dataset.bulk) return bulkAction(btn.dataset.bulk, btn);
    if (btn && btn.dataset.stack) {
      const id = btn.dataset.stack;
      if (S.openStacks.has(id)) S.openStacks.delete(id);
      else S.openStacks.add(id);
      return renderList();
    }
    const item = e.target.closest('.item');
    if (!item || swipeState.moved) return;
    const id = item.dataset.id;
    if (btn && btn.hasAttribute('data-star')) {
      const m = byId(id);
      if (m) setStarred([m], !m.starred);
      return;
    }
    if (btn && btn.hasAttribute('data-check')) return toggleCheck(id);
    if (e.ctrlKey || e.metaKey) return toggleCheck(id);
    if (e.shiftKey) return checkRange(id);
    if (S.checked.size) {
      S.checked.clear();
      selectionChanged();
    }
    S.anchorId = id;
    openMessage(id);
  });
  pane.addEventListener('dblclick', (e) => {
    const item = e.target.closest('.item');
    if (!item || e.target.closest('button')) return;
    const m = byId(item.dataset.id);
    if (m && m.role === 'drafts') compose({ mode: 'draft', id: m.id });
  });
  pane.addEventListener('contextmenu', (e) => {
    const item = e.target.closest('.item');
    if (!item) return;
    e.preventDefault();
    const id = item.dataset.id;
    if (S.checked.size > 1 && S.checked.has(id)) return showMenu(null, bulkMenu(checkedMessages()), { at: { x: e.clientX, y: e.clientY } });
    const m = byId(id);
    if (m) showMenu(null, messageMenu(m), { at: { x: e.clientX, y: e.clientY } });
  });
  pane.addEventListener('dragstart', (e) => {
    const item = e.target.closest('.item');
    if (!item) return;
    const id = item.dataset.id;
    const ids = S.checked.has(id) ? [...S.checked] : [id];
    e.dataTransfer.setData('application/x-rukoo-ids', JSON.stringify(ids));
    // Moved onto a folder, or added to a chat message in the panel.
    e.dataTransfer.effectAllowed = 'copyMove';
    const ghost = document.createElement('div');
    ghost.className = 'drag-ghost';
    ghost.textContent = ids.length === 1 ? (byId(id)?.subject || t('mailbox.drag.one')).slice(0, 60) : t('mailbox.drag.messages', { count: ids.length });
    document.body.appendChild(ghost);
    e.dataTransfer.setDragImage(ghost, -12, -8);
    setTimeout(() => ghost.remove(), 0);
    startMailDrag();
  });
  scroll.addEventListener('pointerdown', () => scroll.classList.remove('kbd'));
  scroll.addEventListener('scroll', () => newRows.size && updateNewPill(), { passive: true });
  bindSwipe(pane);
}

function listAction(action, btn) {
  switch (action) {
    case 'new-mail':
      return showNewRows();
    case 'sync':
      return syncNow();
    case 'search-clear':
      return clearSearch();
    case 'search-everywhere':
      S.searchScope = S.data.accounts.length > 1 && S.scope !== 'all' ? 'account' : 'all';
      return refresh();
    case 'older-load':
      return loadOlder();
    case 'older-retry':
      return retryServerSearch();
    case 'check-all': {
      const list = rows();
      const all = list.every((m) => S.checked.has(m.id));
      S.checked = all ? new Set() : new Set(list.map((m) => m.id));
      return selectionChanged();
    }
    case 'list-more': {
      const compact = S.data.settings.density === 'compact';
      return showMenu(btn, [
        { icon: 'mailOpen', get label() { return t('mailbox.actions.readAll'); }, shortcut: keyLabel('readAll'), action: markAllRead },
        { icon: 'sync', get label() { return t('mailbox.actions.sync'); }, shortcut: keyLabel('sync'), action: syncNow },
        { separator: true },
        { get label() { return t('mailbox.actions.compact'); }, checked: compact, action: () => setDensity(compact ? 'standard' : 'compact') },
        { icon: 'chevronUpDown', get label() { return t('mailbox.sort.menu'); }, action: chooseSort },
        S.view === 'trash' || S.view === 'junk' ? { separator: true } : null,
        S.view === 'trash' || S.view === 'junk' ? { icon: 'trash', label: t('mailbox.emptyFolder.action', { folder: viewLabel() }), action: emptyCurrent, danger: true } : null,
        { separator: true },
        { icon: 'settings', get label() { return t('settings.title.short'); }, action: () => openSettings(ctx) }
      ]);
    }
  }
}

async function setDensity(density) {
  await api('updateSettings', { density });
  refresh();
}

// Marks what the list shows read: the folder, account or category tab, narrowed by a quick filter or a search, with
// the emails folded into stacks. Undo, in the notice or with Ctrl+Z, marks the same emails unread again. When the
// server refuses some, the notice that says so keeps an Undo for the ones it did mark.
async function markAllRead() {
  const list = rows().filter((m) => m.unread && m.role !== 'saved');
  if (!list.length) return toast(t('mailbox.actions.alreadyRead'));
  let undone = false;
  const offerUndo = (message, ids) => {
    const run = () => {
      undone = true;
      setUnread(ids.map((id) => byId(id) || list.find((m) => m.id === id)), true);
    };
    toast(message, { action: { get label() { return t('common.actions.undo'); }, run } });
  };
  offerUndo(t('mailbox.actions.readCount', { count: list.length }), list.map((m) => m.id));
  const { done, error } = await setUnread(list, false, { quiet: true });
  if (!error) return;
  if (!done.length || undone) return toast(error);
  offerUndo(t('mailbox.actions.readSome', { count: done.length, total: list.length, error }), done);
}

export async function syncNow() {
  toast(t('mailbox.sync.syncing'));
  try {
    await api('sync', S.scope === 'all' ? null : S.scope);
    toast(t('mailbox.sync.done'));
  } catch (err) {
    toast(err.message, 5000);
  }
}

async function chooseSort() {
  const v = await choiceDialog(
    t('mailbox.sort.title'),
    [
      { get label() { return t('mailbox.sort.newest'); }, value: 'date-desc' },
      { get label() { return t('mailbox.sort.oldest'); }, value: 'date-asc' },
      { get label() { return t('mailbox.sort.unread'); }, value: 'unread' },
      { get label() { return t('mailbox.sort.sender'); }, value: 'sender' }
    ],
    S.data.settings.sort
  );
  if (v) {
    await api('updateSettings', { sort: v });
    refresh();
  }
}

async function emptyCurrent() {
  const accounts = scopedAccounts();
  const what =
    S.view === 'trash'
      ? t('mailbox.emptyFolder.permanent')
      : t('mailbox.emptyFolder.trash');
  const ok = await confirmDialog(t('mailbox.emptyFolder.title', { folder: viewLabel() }), what, t('mailbox.emptyFolder.confirm'), true);
  if (!ok) return;
  let n = 0;
  for (const a of accounts) {
    const f = a.folders.find((x) => x.role === S.view);
    if (f) n += await api('emptyFolder', a.id, f.path);
  }
  toast(t('mailbox.delete.deletedCount', { count: n }));
}

// ---------- swipe (touch and pen only; a mouse drags messages to folders) ----------

const swipeState = { el: null, startX: 0, startY: 0, dx: 0, moved: false, id: null };
const SWIPE_TRIGGER = 110;

function bindSwipe(pane) {
  pane.addEventListener('pointerdown', (e) => {
    if (!S.data?.settings.swipeActions || e.pointerType === 'mouse' || e.button !== 0) return;
    if (S.data.settings.swipeLeft === 'none' && S.data.settings.swipeRight === 'none') return;
    const item = e.target.closest('.item');
    if (!item || e.target.closest('button')) return;
    Object.assign(swipeState, { el: item, startX: e.clientX, startY: e.clientY, dx: 0, moved: false, id: item.dataset.id, pointer: e.pointerId });
  });
  pane.addEventListener('pointermove', (e) => {
    const st = swipeState;
    if (!st.el || e.pointerId !== st.pointer) return;
    const dx = e.clientX - st.startX;
    const dy = e.clientY - st.startY;
    if (!st.moved) {
      if (Math.abs(dx) < 12 || Math.abs(dx) < Math.abs(dy) * 1.5) return;
      st.moved = true;
      try {
        st.el.setPointerCapture(e.pointerId);
      } catch (_) {
        // Pointer already released; the move still works without capture.
      }
      st.el.style.transition = 'none';
    }
    st.dx = dx;
    st.el.style.transform = `translateX(${dx}px)`;
    const wrap = st.el.parentElement;
    const strength = Math.min(1, Math.abs(dx) / SWIPE_TRIGGER);
    const right = wrap.querySelector('.swipe-bg.right');
    const left = wrap.querySelector('.swipe-bg.left');
    if (right) right.style.opacity = dx > 0 ? strength : 0;
    if (left) left.style.opacity = dx < 0 ? strength : 0;
  });
  const end = (e) => {
    const st = swipeState;
    if (!st.el || e.pointerId !== st.pointer) return;
    const { el, dx, id, moved } = st;
    st.el = null;
    const flush = () => {
      if (st.renderPending) {
        st.renderPending = false;
        renderList();
      }
    };
    if (!moved) return flush();
    el.style.transition = 'transform 200ms cubic-bezier(0.2,0.8,0.2,1)';
    const m = byId(id);
    // The action behind the row on the side it went to, if it went far enough (Settings > Swipe and quick actions).
    const bg = m && Math.abs(dx) >= SWIPE_TRIGGER && el.parentElement.querySelector(`.swipe-bg.${dx > 0 ? 'right' : 'left'}`);
    const action = bg && bg.dataset.swipe;
    if (action === 'archive' || action === 'delete') {
      el.style.transform = `translateX(${dx > 0 ? '' : '-'}110%)`;
      setTimeout(() => rowAction(action, m), 160);
    } else {
      el.style.transform = '';
      el.parentElement.querySelectorAll('.swipe-bg').forEach((b) => (b.style.opacity = 0));
      if (action) rowAction(action, m);
    }
    flush();
    // Swallow the click that follows a drag.
    setTimeout(() => (st.moved = false), 0);
  };
  // Listen on window: a drag released over the reader must still end the swipe.
  window.addEventListener('pointerup', end);
  window.addEventListener('pointercancel', end);
}

// ---------- divider ----------

function bindDivider() {
  const div = $('.divider');
  const set = (w) => {
    prefs.listWidth = Math.round(w);
    root.style.setProperty('--list-w', `${listWidth()}px`);
  };
  div.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    div.setPointerCapture(e.pointerId);
    div.classList.add('dragging');
    const left = $('.listpane').getBoundingClientRect().left;
    const move = (ev) => set(ev.clientX - left);
    const up = () => {
      div.classList.remove('dragging');
      div.removeEventListener('pointermove', move);
      div.removeEventListener('pointerup', up);
      prefs.listWidth = listWidth();
      savePrefs();
    };
    div.addEventListener('pointermove', move);
    div.addEventListener('pointerup', up);
  });
  div.addEventListener('dblclick', () => {
    set(400);
    savePrefs();
  });
  div.addEventListener('keydown', (e) => {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    set(listWidth() + (e.key === 'ArrowRight' ? 24 : -24));
    savePrefs();
  });
}

// ---------- actions ----------

// One call for all of them: main changes its cache at once, so a sync or redraw halfway can't show some as they were.
// Resolves with the ids the server took (done) and its first refusal (error), which shows in a notice unless the
// caller reports it itself (quiet).
async function flagMany(list, flags, quiet) {
  const res = await api('setFlagsMany', list.map((m) => m.id), flags).catch((err) => ({ done: [], error: err.message }));
  if (res.error && !quiet) toast(res.error);
  return res;
}

async function setUnread(list, unread, { quiet = false } = {}) {
  list = list.filter((m) => m.role !== 'saved');
  if (!list.length) return { done: [], error: null };
  for (const m of list) m.unread = unread;
  renderList();
  return flagMany(list, { unread }, quiet);
}

async function setStarred(list, starred) {
  list = list.filter((m) => m.role !== 'saved');
  if (!list.length) return;
  for (const m of list) {
    m.starred = starred;
    if (S.message && S.message.id === m.id) S.message.starred = starred;
  }
  renderList();
  if (S.message && list.some((m) => m.id === S.message.id)) renderReader();
  await flagMany(list, { starred });
}

// The shortcut that marks as read (or unread), or the scheme's toggle when it has one for both.
function readKey(read) {
  return keyLabel(read ? 'read' : 'unread') ? (read ? 'read' : 'unread') : 'toggleRead';
}

function canArchive(m) {
  const acc = account(m.accountId);
  return Boolean(acc && acc.archive && m.role !== 'archive' && m.role !== 'saved' && m.role !== 'drafts');
}

// ---------- row actions: swipes and the buttons on a row under the mouse (Settings > Swipe and quick actions) ----------

const ROW_COLORS = { read: '#2f6fe0', archive: '#1f9d5c', delete: '#d9474b', star: '#d99a00', move: '#7a5af5', category: '#0e9aa7' };

function rowActionAvailable(action, m) {
  if (m.role === 'saved') return action === 'delete';
  if (m.role === 'drafts') return action === 'delete' || action === 'star';
  if (action === 'archive') return canArchive(m);
  if (action === 'category') return Boolean(m.category) && categoriesOn();
  return true;
}

// How an action shows on a row: icon, a short label for a swipe, and a label with its shortcut for a tooltip.
function rowActionLook(action, m) {
  switch (action) {
    case 'read':
      return { icon: m.unread ? icons.mailOpen : icons.markUnread, short: m.unread ? t('mailbox.message.read') : t('mailbox.filters.unread'), title: m.unread ? tip(t('mailbox.actions.read'), readKey(true)) : tip(t('mailbox.actions.unread'), readKey(false)) };
    case 'archive':
      return { icon: icons.archive, short: t('common.actions.archive'), title: tip(t('common.actions.archive'), 'archive') };
    case 'delete':
      return { icon: icons.trash, short: t('common.actions.delete'), title: tip(t('common.actions.delete'), 'delete') };
    case 'star':
      return { icon: m.starred ? icons.starFilled : icons.star, short: m.starred ? t('mailbox.actions.unstar') : t('mailbox.actions.star'), title: tip(m.starred ? t('mailbox.actions.unstar') : t('mailbox.actions.addStar'), 'star') };
    case 'move':
      return { icon: icons.move, short: t('common.actions.move'), title: tip(t('common.actions.move'), 'move') };
    case 'category':
      return { icon: icons.tabs, short: t('mailbox.categories.move'), title: tip(t('mailbox.categories.move'), 'category') };
  }
  return null;
}

function rowAction(action, m) {
  switch (action) {
    case 'read':
      return setUnread([m], !m.unread);
    case 'archive':
      return archiveMessages([m]);
    case 'delete':
      return removeMessages([m]);
    case 'star':
      return setStarred([m], !m.starred);
    case 'move':
      return moveWithPicker([m]);
    case 'category':
      return moveToCategory([m]);
  }
}

// The buttons that cover the time on a row under the mouse.
function quickHtml(m) {
  const acts = (S.data.settings.quickActions || []).filter((a) => rowActionAvailable(a, m));
  if (!acts.length) return '';
  return `<span class="quick">${acts
    .map((a) => {
      const look = rowActionLook(a, m);
      return `<button class="quick-btn ${a === 'star' && m.starred ? 'on' : ''}" data-quick="${a}" tabindex="-1" title="${esc(look.title)}" aria-label="${esc(look.title)}">${look.icon}</button>`;
    })
    .join('')}</span>`;
}

// The color and label behind a row while it is swiped, for each direction that does something.
function swipeHtml(m) {
  const s = S.data.settings;
  if (!s.swipeActions) return '';
  const side = (dir, action) => {
    if (!action || action === 'none' || !rowActionAvailable(action, m)) return '';
    const look = rowActionLook(action, m);
    return `<div class="swipe-bg ${dir}" data-swipe="${action}" style="--sw:${ROW_COLORS[action]}">${look.icon}<span>${esc(look.short)}</span></div>`;
  };
  return side('right', s.swipeRight) + side('left', s.swipeLeft);
}

// Takes messages out of the list right away and opens the next one if the open message left.
function detach(list) {
  const ids = new Set(list.map((m) => m.id));
  const visible = visibleRows();
  let next = null;
  const wasOpen = ids.has(S.selectedId);
  if (wasOpen) {
    const i = visible.findIndex((x) => x.id === S.selectedId);
    next = visible.slice(i + 1).find((x) => !ids.has(x.id)) || visible.slice(0, Math.max(0, i)).reverse().find((x) => !ids.has(x.id)) || null;
  }
  S.list = S.list.filter((x) => !ids.has(x.id));
  for (const id of ids) S.checked.delete(id);
  if (wasOpen) {
    if (next) openMessage(next.id);
    else closeReader();
  }
  renderList();
  if (!wasOpen && S.checked.size < 2 && $('.reader .multi')) renderReader();
  return wasOpen;
}

async function offerUndo(ids, message, reopen) {
  const can = await Promise.all(ids.map((id) => api('canUndo', id).catch(() => false)));
  const undoable = ids.filter((_, i) => can[i]);
  if (!undoable.length) return toast(message);
  toast(message, { action: { get label() { return t('common.actions.undo'); }, run: () => undoMoves(undoable, reopen) } });
}

async function undoMoves(ids, reopen) {
  const restored = [];
  for (const id of ids) {
    try {
      restored.push(await api('undoMove', id));
    } catch (err) {
      toast(err.message, 5000);
    }
  }
  await refresh();
  const back = restored.filter(Boolean);
  if (reopen && back.length === 1) openMessage(back[0]);
  if (back.length) toast(back.length === 1 ? t('mailbox.undo.restored') : t('mailbox.undo.restoredCount', { count: back.length }));
}

export async function removeMessages(list) {
  list = list.filter(Boolean);
  if (!list.length) return;
  const permanent = list.filter((m) => m.role === 'trash' || m.role === 'saved');
  if (permanent.length) {
    const ok = await confirmDialog(
      t('mailbox.delete.title'),
      permanent.length === 1 ? t('mailbox.delete.one') : t('mailbox.delete.permanentCount', { count: permanent.length }),
      t('common.actions.delete'),
      true
    );
    if (!ok) return renderList();
  }
  const reopen = detach(list);
  const moved = [];
  const confirm = [];
  let deleted = 0;
  for (const m of list) {
    try {
      const where = await api('remove', m.id);
      if (where === 'trash') moved.push(m.id);
      else if (where === 'confirm') confirm.push(m);
      else deleted++;
    } catch (err) {
      toast(err.message, 5000);
      refresh();
    }
  }
  if (confirm.length) {
    const ok = await confirmDialog(
      t('mailbox.delete.title'),
      confirm.length === 1
        ? t('mailbox.delete.noTrash')
        : t('mailbox.delete.noTrashCount', { count: confirm.length }),
      t('common.actions.delete'),
      true
    );
    if (!ok) return refresh();
    for (const m of confirm) {
      try {
        await api('remove', m.id, { force: true });
        deleted++;
      } catch (err) {
        toast(err.message, 5000);
      }
    }
  }
  if (moved.length) offerUndo(moved, moved.length === 1 ? t('mailbox.delete.movedOne') : t('mailbox.delete.movedCount', { count: moved.length }), reopen);
  else if (deleted) toast(deleted === 1 ? t('mailbox.delete.deletedOne') : t('mailbox.delete.deletedCount', { count: deleted }));
}

function destination(m, target) {
  const acc = account(m.accountId);
  if (!acc) return null;
  if (target.path) return m.accountId === target.accountId ? target.path : null;
  if (target.role === 'archive') return acc.archive;
  return (acc.folders.find((f) => f.role === target.role) || {}).path || null;
}

async function moveMessages(list, target, label) {
  const plan = list
    .filter((m) => m && m.role !== 'saved')
    .map((m) => ({ m, path: destination(m, target) }))
    .filter((p) => p.path && p.path !== p.m.folder);
  if (!plan.length) return toast(target.role === 'archive' ? t('mailbox.move.noArchive') : t('mailbox.move.alreadyThere'));
  const reopen = detach(plan.map((p) => p.m));
  const moved = [];
  for (const { m, path } of plan) {
    try {
      await api('move', m.id, path);
      moved.push(m.id);
    } catch (err) {
      toast(err.message, 5000);
      refresh();
    }
  }
  if (moved.length) offerUndo(moved, moved.length === 1 ? t('mailbox.move.one', { folder: label }) : t('mailbox.move.count', { count: moved.length, folder: label }), reopen);
}

function archiveMessages(list) {
  return moveMessages(list, { role: 'archive' }, t('mailbox.folders.archive'));
}

async function pickFolder(list) {
  const accountIds = [...new Set(list.map((m) => m.accountId))];
  const here = new Set(list.map((m) => m.folder));
  let options;
  if (accountIds.length === 1) {
    const acc = account(accountIds[0]);
    options = acc.folders
      .filter((f) => f.role !== 'drafts' && f.role !== 'sent' && !(here.size === 1 && here.has(f.path)))
      .map((f) => ({ label: f.name, value: f.role ? { role: f.role, label: f.name } : { path: f.path, accountId: acc.id, label: f.name } }));
  } else {
    options = ['inbox', 'archive', 'junk', 'trash'].map((role) => ({ label: LABELS[role](), value: { role, label: LABELS[role]() } }));
  }
  return choiceDialog(list.length === 1 ? t('mailbox.move.title') : t('mailbox.move.countTitle', { count: list.length }), options, null);
}

// Puts emails in another inbox category. Nothing changes on the mail server; the toast offers to do the same for
// their senders' later mail.
async function moveToCategory(list) {
  list = list.filter((m) => m && m.category);
  if (!list.length || !S.categories) return;
  const options = S.categories.categories.map((c) => ({ value: c.id, label: c.name }));
  const same = list.every((m) => m.category === list[0].category);
  const id = await choiceDialog(t('mailbox.categories.pick'), options, same ? list[0].category : null);
  if (!id) return;
  const ids = list.map((m) => m.id);
  try {
    await api('categoriesMove', ids, id, {});
  } catch (err) {
    return toast(err.message, 5000);
  }
  const name = (categoryOf(id) || {}).name || '';
  const senders = new Set(list.map((m) => String((m.from && m.from.address) || '').toLowerCase()).filter(Boolean));
  toast(t('mailbox.categories.moved', { name }), {
    action: {
      label: t('mailbox.categories.always'),
      // Ctrl+Z after a move means undo, which this isn't.
      shortcut: false,
      run: async () => {
        try {
          await api('categoriesMove', ids, id, { sender: true });
        } catch (err) {
          return toast(err.message, 5000);
        }
        toast(senders.size === 1 ? t('mailbox.categories.alwaysDone', { sender: person(list[0].from), name }) : t('mailbox.categories.alwaysDoneMany', { count: senders.size, name }));
      }
    }
  });
}

async function moveWithPicker(list) {
  list = list.filter((m) => m && m.role !== 'saved');
  if (!list.length) return;
  const target = await pickFolder(list);
  if (!target) return;
  if (target.role === 'trash') return removeMessages(list);
  return moveMessages(list, target, target.label);
}

function bulkAction(action, btn) {
  const list = checkedMessages();
  switch (action) {
    case 'read':
      return setUnread(list, false);
    case 'unread':
      return setUnread(list, true);
    case 'star':
      return setStarred(list, !list.every((m) => m.starred));
    case 'move':
      return moveWithPicker(list);
    case 'archive':
      return archiveMessages(list);
    case 'delete':
      return removeMessages(list);
    case 'clear':
      return clearChecks();
    case 'more':
      return showMenu(btn, bulkMenu(list));
  }
}

function bulkMenu(list) {
  const anyUnread = list.some((m) => m.unread);
  const canArchive = list.some((m) => account(m.accountId)?.archive && m.role !== 'archive');
  return [
    { icon: anyUnread ? 'mailOpen' : 'markUnread', label: anyUnread ? t('mailbox.actions.read') : t('mailbox.actions.unread'), shortcut: keyLabel(readKey(anyUnread)), action: () => setUnread(list, !anyUnread) },
    { icon: 'star', label: list.every((m) => m.starred) ? t('mailbox.actions.unstar') : t('mailbox.actions.addStar'), shortcut: keyLabel('star'), action: () => setStarred(list, !list.every((m) => m.starred)) },
    addToChatItem(list),
    { separator: true },
    { icon: 'move', get label() { return t('mailbox.actions.moveMenu'); }, shortcut: keyLabel('move'), action: () => moveWithPicker(list) },
    list.some((m) => m.category) ? { icon: 'tabs', get label() { return t('mailbox.categories.move'); }, shortcut: keyLabel('category'), action: () => moveToCategory(list) } : null,
    canArchive ? { icon: 'archive', get label() { return t('common.actions.archive'); }, shortcut: keyLabel('archive'), action: () => archiveMessages(list) } : null,
    { separator: true },
    { icon: 'trash', label: t('mailbox.delete.countAction', { count: list.length }), shortcut: keyLabel('delete'), action: () => removeMessages(list), danger: true }
  ];
}

function messageMenu(m) {
  const saved = m.role === 'saved';
  const draft = m.role === 'drafts';
  const acc = account(m.accountId);
  const canArchive = Boolean(acc && acc.archive && m.role !== 'archive' && !saved && !draft);
  return [
    draft ? { icon: 'edit', get label() { return t('composer.actions.editDraft'); }, shortcut: 'Enter', action: () => compose({ mode: 'draft', id: m.id }) } : null,
    draft ? null : { icon: 'reply', get label() { return t('composer.titles.reply'); }, shortcut: keyLabel('reply'), action: () => compose({ mode: 'reply', id: m.id }) },
    draft ? null : { icon: 'replyAll', get label() { return t('composer.titles.replyAll'); }, shortcut: keyLabel('replyAll'), action: () => compose({ mode: 'replyAll', id: m.id }) },
    draft ? null : { icon: 'forward', get label() { return t('composer.titles.forward'); }, shortcut: keyLabel('forward'), action: () => compose({ mode: 'forward', id: m.id }) },
    addToChatItem([m]),
    { separator: true },
    saved ? null : { icon: m.unread ? 'mailOpen' : 'markUnread', label: m.unread ? t('mailbox.actions.read') : t('mailbox.actions.unread'), shortcut: keyLabel(readKey(m.unread)), action: () => setUnread([m], !m.unread) },
    saved ? null : { icon: 'star', label: m.starred ? t('mailbox.actions.unstar') : t('mailbox.actions.addStar'), shortcut: keyLabel('star'), action: () => setStarred([m], !m.starred) },
    saved ? null : { icon: 'move', get label() { return t('mailbox.actions.moveMenu'); }, shortcut: keyLabel('move'), action: () => moveWithPicker([m]) },
    m.category ? { icon: 'tabs', get label() { return t('mailbox.categories.move'); }, shortcut: keyLabel('category'), action: () => moveToCategory([m]) } : null,
    canArchive ? { icon: 'archive', get label() { return t('common.actions.archive'); }, shortcut: keyLabel('archive'), action: () => archiveMessages([m]) } : null,
    { separator: true },
    {
      icon: 'vip',
      label: m.vip ? t('mailbox.vip.remove') : t('mailbox.vip.add'),
      action: async () => {
        const on = await api('toggleVip', m.from.address);
        toast(on ? t('mailbox.vip.addedSender', { sender: person(m.from) }) : t('mailbox.vip.removedSender', { sender: person(m.from) }));
      }
    },
    saved || m.role === 'sent' || draft
      ? null
      : {
          icon: 'ban',
          get label() { return t('mailbox.spam.add'); },
          action: async () => {
            const ok = await confirmDialog(t('mailbox.spam.confirm'), t('mailbox.spam.confirmHelp', { address: m.from.address }), t('common.actions.add'));
            if (!ok) return;
            if (S.selectedId === m.id) closeReader();
            try {
              await api('markSpam', m.id);
              toast(t('mailbox.spam.added'));
            } catch (err) {
              toast(err.message, 5000);
            }
          }
        },
    saved
      ? null
      : {
          icon: 'saved',
          get label() { return t('mailbox.saved.save'); },
          action: async () => {
            await api('saveToDevice', m.id);
            toast(t('mailbox.saved.saved'));
          }
        },
    {
      icon: 'export',
      get label() { return t('mailbox.export.action'); },
      action: async () => {
        const p = await api('exportEml', m.id).catch((err) => toast(err.message));
        if (p) toast(t('mailbox.export.done'));
      }
    },
    { icon: 'print', get label() { return t('mailbox.actions.print'); }, shortcut: keyLabel('print'), action: () => printMessage(m.id) },
    { separator: true },
    { icon: 'trash', get label() { return t('common.actions.delete'); }, shortcut: keyLabel('delete'), action: () => removeMessages([m]), danger: true }
  ];
}

async function printMessage(id) {
  // The reader's copy, unless it is still waiting for its inline pictures.
  const full = S.message && S.message.id === id && S.message.html != null && !S.message.partial ? S.message : await api('get', id);
  const head = `<h2 style="font:600 20px Segoe UI,sans-serif;margin:0 0 8px">${esc(full.subject)}</h2>
    <div style="font:13px Segoe UI,sans-serif;color:#444;margin-bottom:16px">${esc(t('reader.headers.from'))}: ${esc(formatAddress(full.from))}<br>${esc(t('reader.headers.to'))}: ${esc(full.to.map(formatAddress).join(', '))}<br>${esc(t('reader.headers.date'))}: ${esc(longDate(full.date))}</div><hr>`;
  await api('print', `<!doctype html><meta charset="utf-8"><body style="margin:24px">${head}${full.html}</body>`);
}

// Writing happens in the reading pane; the pop-out button moves it to a window of its own.
// Each request to write (or to open a message) takes a number; a request that a newer one overtook
// while it waited stops, so a slow reply can never replace the message you started after it.
// still: the caller's check after each wait that its reason to write holds (the agent panel: whether its chat is
// still on screen); without it the request stops. claim: asked once the reading pane is free, right before the editor
// takes its place; false stops the request (a refused email that opened elsewhere meanwhile, reopenRefused).
let composeTurn = 0;

async function compose(opts) {
  if (!S.data || !S.data.accounts.length) return;
  const turn = ++composeTurn;
  if (opts.mode === 'draft' && S.composer && S.composer.draftId === opts.id) return S.composer.focus();
  // A draft that is open in a compose window stays there.
  if (opts.mode === 'draft' && (await api('draftWindow', opts.id).catch(() => false))) return;
  if (turn !== composeTurn) return;
  const accountId = S.scope !== 'all' ? S.scope : null;
  const { still, claim, ...rest } = opts;
  // An editor that opens again after Undo send brings its own mode, account and the email it answers.
  const restore = rest.restore || null;
  const full = restore ? { accountId, ...restoreOpts(restore), restore, sendError: rest.sendError || null } : { accountId, ...rest };
  const holds = () => !still || still();
  let message = restore ? restore.original || null : null;
  if (full.id && !restore) {
    try {
      message = await api('get', full.id);
    } catch (err) {
      if (turn === composeTurn) toast(err.message, 5000);
      return;
    }
    if (turn !== composeTurn) return;
  }
  if (!holds()) return;
  // Whatever is being written now is kept as a draft before the new message takes its place.
  if (S.composer && !(await S.composer.leave())) return;
  if (turn !== composeTurn || S.composer || !holds()) return;
  if (claim && (!(await claim()) || turn !== composeTurn || S.composer || !holds())) return;
  const host = $('.reader');
  host.innerHTML = '';
  S.expanded = false;
  let mounted = null;
  S.composer = mounted = mountComposer(host, {
    data: S.data,
    opts: full,
    message,
    inline: true,
    onDone: () => {
      S.composer = null;
      // The chat's draft cards remember which stored draft this composer left behind.
      agentPanel?.composerClosed(mounted);
      // An email that left the list while the editor was open doesn't come back with the reading pane.
      reconcileReader();
      renderShell();
      renderReader();
      agentViewChanged();
    },
    onPopOut: (o) => api('openCompose', { accountId, ...o }).catch((err) => toast(err.message, 5000))
  });
  renderShell();
  agentViewChanged();
  return S.composer;
}

// The mode, email and account an editor that opens again had (composer.js undoState).
function restoreOpts(restore) {
  const o = restore.opts && typeof restore.opts === 'object' ? restore.opts : {};
  return { mode: typeof o.mode === 'string' ? o.mode : 'new', id: typeof o.id === 'string' ? o.id : null, ...(typeof o.accountId === 'string' ? { accountId: o.accountId } : {}) };
}

// ---------- undo send ----------

// After Send, while the email waits in main's outbox (Settings > General > Undo send). Undo, or Ctrl+Z, takes it back;
// a newer notice takes this one's place, and the email then goes when its time is up. "Email sent" follows from
// main ('send-done'). undoNotice: the email whose notice shows, and its action.
let undoNotice = null;
function offerUndoSend({ id, ms }) {
  const action = { get label() { return t('common.actions.undo'); }, run: () => undoSend(id) };
  undoNotice = { id, action };
  toast(t('composer.undoSend.queued'), { ms: Number(ms) || undefined, action });
}

// The emails Don't quit kept waiting (askQuitWithHeldMail in main.js). Their clocks stand still until this page asks
// for their notices, so none runs out before its Undo shows: asked when told ('send-paused'), and once the first
// state is drawn, for a main window that was still loading or was made anew after the last window closed.
async function takeUndoNotices() {
  if (!S.data) return;
  for (const notice of await api('undoNotices').catch(() => [])) offerUndoSend(notice);
}

// An email that waited went. A newer email's notice keeps its Undo instead of making way for "Email sent".
function waitedMailSent({ id }) {
  if (undoNotice && undoNotice.id !== id && showingToast(undoNotice.action)) return;
  toast(t('composer.status.sent'));
}

// The emails the server refused after their wait, which main holds for this window until their editors are open
// (refused in main.js): asked for when told ('send-failed'), and once the first state is drawn, so a window that was
// still loading when an email failed opens it too. Each one's notice goes, and its editor opens again with the error:
// the first in the reading pane, any others in compose windows (main opens those), so none of them pushes another
// out. Main lets each go only once it is open, so the ones this page dies before opening still open elsewhere. One
// round at a time: a second 'send-failed' asks after the first round, and finds only what is new.
let takingFailures = Promise.resolve();
function takeSendFailures() {
  takingFailures = takingFailures.then(takeSendFailuresNow, takeSendFailuresNow);
  return takingFailures;
}

async function takeSendFailuresNow() {
  if (!S.data || !$('.reader')) return;
  const list = await api('sendFailures').catch(() => []);
  for (const [i, { id, restore, error }] of list.entries()) {
    if (!restore || typeof restore !== 'object') continue;
    if (undoNotice && undoNotice.id === id) dropToast(undoNotice.action);
    if (i === 0) await reopenRefused(id, restore, String(error || ''));
    else await api('openRefused', id).catch((err) => toast(err.message, 5000));
  }
}

// A refused email back in the reading pane. It is claimed only once the pane is free, after the editor there saved
// its text, so main still has it if this page dies during that save, and the pane leaves it alone when main opened
// it elsewhere meanwhile. Main lets it go once the editor is open; when the pane can't take it, main opens it in a
// compose window.
async function reopenRefused(id, restore, sendError) {
  setMode('mail');
  const claim = () => api('claimRefused', id).catch(() => false);
  const editor = await compose({ restore, sendError, claim }).catch(() => null);
  if (editor) await api('refusedOpened', id).catch(() => {});
  else await api('openRefused', id).catch((err) => toast(err.message, 5000));
}

async function undoSend(id) {
  let res;
  try {
    res = await api('undoSend', id);
  } catch (err) {
    return toast(err.message, 5000);
  }
  if (!res || res.late) return toast(t('composer.undoSend.late'));
  // An email written in a compose window opens in a new one; main did that.
  if (res.restore) await reopenComposer(res.restore);
  toast(t('composer.undoSend.undone'));
}

// The editor of an email that waited, back in the reading pane. When the pane can't take it (its own editor is busy
// asking or saving), it opens in a compose window, so the email is never lost. sendError: why the server refused it.
async function reopenComposer(restore, sendError = null) {
  setMode('mail');
  const editor = await compose({ restore, sendError }).catch(() => null);
  if (editor) return;
  await api('openCompose', { ...restoreOpts(restore), restore, sendError }).catch((err) => toast(err.message, 5000));
}

// ---------- reader ----------

// auto: shown without being asked for (the newest message on load); it stays unread. known: the message as a related
// email under another one, for one the list doesn't have, such as a reply in Sent; it opens and is read like a row.
export async function openMessage(id, { auto = false, known = null } = {}) {
  // Opening a message cancels a reply or forward that is still loading.
  const turn = ++composeTurn;
  if (S.composer && !(await S.composer.leave())) return;
  if (turn !== composeTurn) return;
  const listed = S.list.find((x) => x.id === id);
  const m = listed || (known && known.id === id ? known : null);
  // An inbox email the list leaves out, from a notification or the agent, opens in its own category, so its row is
  // on screen and the reading pane keeps it (reconcileReader).
  if (listed && categoriesOn() && !categoryRows().includes(m) && categoryOf(m.category)) {
    S.category = m.category;
    S.filter = 'all';
    clearChecks();
    renderSidebar();
    renderList();
  }
  const left = S.keptId && S.keptId !== id ? S.keptId : null;
  if (S.keptId !== id) S.keptId = m && m.unread ? id : null;
  S.selectedId = id;
  S.anchorId = id;
  S.cursorId = id;
  S.details = false;
  S.loading = true;
  S.message = m ? { ...m, html: null, attachments: [] } : S.message;
  syncRowClasses();
  if (left) letGo(left);
  revealRow(id);
  renderShell();
  renderReader();
  agentViewChanged();
  try {
    const full = await api('get', id, { fast: true });
    if (S.selectedId !== id) return;
    // The related emails come with the message, so the reading pane draws once. They need its References.
    const related = await api('related', id, full.references || []).catch(() => []);
    if (S.selectedId !== id) return;
    S.message = full;
    S.related = { id, list: related };
  } catch (err) {
    if (S.selectedId !== id) return;
    S.message = { ...(m || {}), error: err.message };
  } finally {
    if (S.selectedId === id) S.loading = false;
  }
  renderReader();
  // Again with the full message: the panel needs its Message-ID and whether it can unsubscribe.
  agentViewChanged();
  if (m && m.unread && m.role !== 'drafts' && !auto) setUnread([m], false);
  if (S.message && S.message.id === id && S.message.partial) completeMessage(id);
}

// A large message shows its text before its attachments are in (engine.js readableSource). Once the whole message is
// on disk it is read again: the attachments get their exact sizes and thumbnails, and inline pictures that didn't come
// with the text appear. One that came in but couldn't be kept, on a full disk, is read whole once more in the user's
// turn, as the reader reads any message it can't keep. kept is what downloaded said, when the caller already asked.
// The email itself only redraws when it changed, and keeps its scroll position then.
async function completeMessage(id, kept) {
  const still = () => S.selectedId === id && S.message && S.message.id === id && S.message.partial;
  // null when the download failed: the reader stays as it is until an attachment is opened (wholeMessage).
  if (kept === undefined) kept = await api('downloaded', id).catch(() => null);
  if (kept == null || !still()) return;
  const full = await (kept ? api('get', id, { fast: true }) : api('get', id)).catch(() => null);
  if (!full || full.partial || !still()) return;
  const before = S.message;
  // With the flags the reader shows, which a star or read change on its way may set ahead of main.
  S.message = { ...full, unread: before.unread, starred: before.starred };
  if (S.composer || S.checked.size > 1) return;
  if (full.html !== before.html) {
    const top = $('.reader .reader-scroll')?.scrollTop || 0;
    renderReader();
    const scroll = $('.reader .reader-scroll');
    if (scroll) scroll.scrollTop = top;
    return;
  }
  const atts = $('.reader .atts');
  if (atts) atts.outerHTML = attachmentsHtml(S.message);
  loadPreviews(S.message);
}

// Attachment buttons waiting for their message to come in, by message id and attachment index (or 'all' for Save
// all). They show a spinner for their icon.
const fetchingAtts = new Set();

function attachmentSpinner(id, key, on) {
  const k = `${id}|${key}`;
  if (on) fetchingAtts.add(k);
  else fetchingAtts.delete(k);
  if (!S.message || S.message.id !== id) return;
  const btn = key === 'all' ? $('.reader [data-reader="save-all"]') : $(`.reader [data-save-att="${key}"]`);
  if (!btn) return;
  btn.setAttribute('aria-busy', String(on));
  const icon = document.createElement('template');
  icon.innerHTML = on ? '<span class="spinner"></span>' : icons.download;
  btn.querySelector('svg, .spinner')?.replaceWith(icon.content);
}

// An attachment of a message whose text came first is only there once the whole message is: that comes in the user's
// turn now. The button shows a spinner meanwhile, after a moment, so a quick download doesn't flash. The reader then
// completes too, also when the background download had failed.
async function wholeMessage(m, key) {
  if (!m.partial) return;
  const timer = setTimeout(() => attachmentSpinner(m.id, key, true), 100);
  let kept = false;
  try {
    kept = await api('downloaded', m.id, { now: true });
  } finally {
    clearTimeout(timer);
    if (fetchingAtts.has(`${m.id}|${key}`)) attachmentSpinner(m.id, key, false);
  }
  if (S.message && S.message.id === m.id && S.message.partial) completeMessage(m.id, kept);
}

export function closeReader() {
  S.selectedId = null;
  S.message = null;
  S.expanded = false;
  renderShell();
  renderReader();
  syncRowClasses();
  agentViewChanged();
}

function renderReaderNav() {
  const list = visibleRows();
  const i = list.findIndex((x) => x.id === S.selectedId);
  const up = $('[data-reader="prev"]');
  const down = $('[data-reader="next"]');
  if (up) up.disabled = i <= 0;
  if (down) down.disabled = i < 0 || i >= list.length - 1;
}

function attachmentKind(name) {
  const ext = (String(name).match(/\.([a-z0-9]{1,5})$/i) || [])[1]?.toLowerCase() || '';
  const kinds = {
    pdf: 'pdf',
    doc: 'doc', docx: 'doc', odt: 'doc', rtf: 'doc', txt: 'doc',
    xls: 'sheet', xlsx: 'sheet', csv: 'sheet', ods: 'sheet',
    ppt: 'slides', pptx: 'slides', key: 'slides',
    png: 'image', jpg: 'image', jpeg: 'image', gif: 'image', webp: 'image', heic: 'image', svg: 'image',
    zip: 'archive', rar: 'archive', '7z': 'archive', gz: 'archive',
    ics: 'cal', eml: 'mail', msg: 'mail'
  };
  return { ext: ext ? ext.toUpperCase().slice(0, 4) : 'FILE', kind: kinds[ext] || 'other' };
}

function ownAddresses() {
  return new Set(S.data.accounts.flatMap((a) => [a.email, ...a.identities.map((i) => i.address)]).map((x) => String(x).toLowerCase()));
}

// "aan mij, Joris Bakker"; the chevron opens the full header.
function recipientsHtml(m) {
  const own = ownAddresses();
  const all = [...(m.to || []), ...(m.cc || []), ...(m.bcc || [])];
  const names = all.map((a) => (own.has(String(a.address).toLowerCase()) ? t('reader.recipients.me') : person(a)));
  const unique = [...new Set(names)];
  const shown = unique.slice(0, 3).join(', ') + (unique.length > 3 ? t('reader.recipients.others', { count: unique.length - 3 }) : '');
  return `<button class="to-summary" data-reader="details" aria-expanded="${S.details}" title="${esc(t(S.details ? 'reader.details.hide' : 'reader.details.show'))}">${
    all.length ? t('reader.recipients.summary', { recipients: esc(shown) }) : t('reader.recipients.unknown')
  }${icons.chevronDown}</button>`;
}

function detailsHtml(m) {
  if (!S.details) return '';
  const people = (list) =>
    (list || [])
      .map((a) => `<button class="person" data-person="${esc(a.address)}" data-name="${esc(a.name || '')}">${esc(formatAddress(a))}</button>`)
      .join(', ');
  const from = m.from || {};
  const replyTo = (m.replyTo || []).filter((a) => String(a.address).toLowerCase() !== String(from.address || '').toLowerCase());
  const row = (k, v) => (v ? `<span class="k">${k}</span><span class="v">${v}</span>` : '');
  return `<div class="details">
    ${row(t('reader.headers.from'), people([from]))}
    ${row(t('reader.headers.to'), people(m.to))}
    ${row(t('reader.headers.cc'), people(m.cc))}
    ${row(t('reader.headers.bcc'), people(m.bcc))}
    ${row(t('reader.headers.replyTo'), people(replyTo))}
    ${row(t('reader.headers.date'), esc(longDate(m.date)))}
  </div>`;
}

const previews = new Map();

// One attachment is just its card; several get a header with a save-all action. Images show a thumbnail.
function attachmentsHtml(m) {
  // An invitation's calendar part shows as its card (invite.js).
  const list = (m.attachments || []).filter((a) => !(a.calendar && m.invite));
  if (!list.length) return '';
  const total = list.reduce((n, a) => n + (a.size || 0), 0);
  const open = list.length === 1 || S.attachmentsOpen;
  // The download icon, or a spinner while the button waits for its message (wholeMessage).
  const busy = (key) => fetchingAtts.has(`${m.id}|${key}`);
  const saveIcon = (key) => (busy(key) ? '<span class="spinner"></span>' : icons.download);
  const save = (a) =>
    `<button class="icon-btn sm" data-save-att="${a.index}" aria-busy="${busy(a.index)}" data-i18n-title="common.actions.save" title="${esc(t('common.actions.save'))}">${saveIcon(a.index)}</button>`;
  const card = (a) => {
    const { ext, kind } = attachmentKind(a.filename);
    const thumb = previews.get(`${m.id}|${a.index}`);
    if (kind === 'image') {
      return `<div class="att image" data-preview="${a.index}">
        <button class="att-open" data-open-att="${a.index}" title="${esc(t('reader.attachments.open', { filename: a.filename }))}"><span class="thumb">${thumb ? `<img src="${thumb}" alt="">` : `<span class="ftype image">${esc(ext)}</span>`}</span>
        <span class="att-text"><span class="att-name">${esc(a.filename)}</span><span class="att-size">${fileSize(a.size)}</span></span></button>
        ${save(a)}
      </div>`;
    }
    return `<div class="att">
      <button class="att-open" data-open-att="${a.index}" title="${esc(t('reader.attachments.open', { filename: a.filename }))}"><span class="ftype ${kind}">${esc(ext)}</span><span class="att-text"><span class="att-name">${esc(a.filename)}</span><span class="att-size">${fileSize(a.size)}</span></span></button>
      ${save(a)}
    </div>`;
  };
  const head =
    list.length > 1
      ? `<div class="atts-head">
          <button class="atts-toggle" data-reader="toggle-atts" aria-expanded="${open}">${icons.chevronDown}<span>${esc(t('reader.attachments.count', { count: list.length }))}</span><span class="atts-size">${fileSize(total)}</span></button>
          <button class="link-btn" data-reader="save-all" aria-busy="${busy('all')}">${saveIcon('all')}<span data-i18n="reader.attachments.saveAll">${esc(t('reader.attachments.saveAll'))}</span></button>
        </div>`
      : '';
  return `<section class="atts ${open ? '' : 'closed'}" data-i18n-aria-label="mailbox.filters.attachments" aria-label="${esc(t('mailbox.filters.attachments'))}">${head}${open ? `<div class="atts-list">${list.map(card).join('')}</div>` : ''}</section>`;
}

// A subject without Re: and Fwd:, the way main compares subjects (mailutil.js baseSubject).
function baseSubject(subject) {
  let s = String(subject || '').trim();
  for (let prev = null; prev !== s; ) {
    prev = s;
    s = s.replace(/^(?:re|fwd?|fw|antw|aw|doorst)(?:\s*\[\d+\])?\s*:\s*/i, '').trim();
  }
  return s.toLowerCase().replace(/\s+/g, ' ');
}

// How many related emails show before "Show all".
const RELATED_SHOWN = 4;

// The rest of the conversation under the open email, newest first (engine.js related), so the list itself stays one
// row per email. A row names the sender and what the email says; its subject when that differs from this one's, and
// its folder or account when that isn't this one's. A click opens it.
function relatedHtml(m) {
  const list = S.related && S.related.id === m.id ? S.related.list : [];
  if (!list.length) return '';
  const open = prefs.relatedOpen !== false;
  const all = S.relatedAll === m.id || list.length <= RELATED_SHOWN + 1;
  const own = ownAddresses();
  const topic = baseSubject(m.subject);
  const row = (r) => {
    const who = own.has(String((r.from && r.from.address) || '').toLowerCase()) ? t('reader.related.you') : person(r.from) || t('mailbox.message.unknownSender');
    const text = (baseSubject(r.subject) === topic && cleanPreview(r.preview)) || r.subject || t('mailbox.message.noSubject');
    const acc = r.accountId !== m.accountId && account(r.accountId);
    const where = acc ? accountLabel(acc) : r.folder !== m.folder ? folderName(r) : '';
    const draft = r.role === 'drafts' ? `<span class="draft-tag" data-i18n="composer.titles.draft">${esc(t('composer.titles.draft'))}</span>` : '';
    return `<button class="rel ${r.unread ? 'unread' : ''}" data-related="${esc(r.id)}" title="${esc(r.subject || t('mailbox.message.noSubject'))}">
      <span class="rel-dot" aria-hidden="true"></span>
      <span class="rel-who">${draft}${esc(who)}</span>
      <span class="rel-text">${esc(text)}</span>
      ${where ? `<span class="pill">${esc(where)}</span>` : ''}
      <time class="rel-date">${listTime(r.date)}</time>
    </button>`;
  };
  const more = all
    ? ''
    : `<button class="link-btn rel-more" data-reader="related-all">${esc(t('reader.related.showAll', { count: list.length }))}</button>`;
  return `<section class="related ${open ? '' : 'closed'}" data-i18n-aria-label="reader.related.label" aria-label="${esc(t('reader.related.label'))}">
    <div class="rel-head"><button class="rel-toggle" data-reader="toggle-related" aria-expanded="${open}">${icons.chevronDown}<span>${esc(t('reader.related.count', { count: list.length }))}</span></button></div>
    <div class="rel-body" ${open ? '' : 'inert'}><div class="rel-clip"><div class="rel-list">${(all ? list : list.slice(0, RELATED_SHOWN)).map(row).join('')}</div>${more}</div></div>
  </section>`;
}

async function loadPreviews(m) {
  for (const a of m.attachments || []) {
    const key = `${m.id}|${a.index}`;
    if (attachmentKind(a.filename).kind !== 'image' || previews.has(key)) continue;
    previews.set(key, null);
    const url = await api('attachmentPreview', m.id, a.index).catch(() => null);
    previews.set(key, url);
    const el = url && S.message && S.message.id === m.id && $(`.att.image[data-preview="${a.index}"] .thumb`);
    if (el) el.innerHTML = `<img src="${url}" alt="">`;
  }
}

function readerBar(m) {
  const draft = m.role === 'drafts';
  const saved = m.role === 'saved';
  const acc = account(m.accountId);
  const canArchive = Boolean(acc && acc.archive && m.role !== 'archive' && !saved && !draft);
  const nav = `<span class="bar-subject" aria-hidden="true">${esc(m.subject || '')}</span>
    <span class="spacer"></span>
    <button class="icon-btn" data-reader="prev" data-i18n-title="reader.navigation.previous" title="${esc(t('reader.navigation.previous'))}">${icons.up}</button>
    <button class="icon-btn" data-reader="next" data-i18n-title="reader.navigation.next" title="${esc(t('reader.navigation.next'))}">${icons.down}</button>
    <button class="icon-btn wide-only" data-reader="expand" title="${S.expanded ? t('reader.navigation.showList') : t('reader.navigation.expand')}">${S.expanded ? icons.collapse : icons.expand}</button>`;
  const back = `<button class="icon-btn narrow-only" data-reader="back" data-i18n-title="reader.navigation.back" title="${esc(t('reader.navigation.back'))}">${icons.back}</button>`;
  if (draft) {
    return `<div class="reader-bar" role="toolbar" data-i18n-aria-label="reader.actions.label" aria-label="${esc(t('reader.actions.label'))}">${back}
      <button class="tbtn primary" data-reader="edit" data-i18n-title="reader.actions.editDraftShortcut" title="${esc(t('reader.actions.editDraftShortcut'))}">${icons.edit}<span data-i18n="reader.actions.edit">${esc(t('reader.actions.edit'))}</span></button>
      <button class="icon-btn" data-reader="delete" title="${esc(tip(t('common.actions.delete'), 'delete'))}">${icons.trash}</button>
      ${nav}</div>`;
  }
  return `<div class="reader-bar" role="toolbar" data-i18n-aria-label="reader.actions.label" aria-label="${esc(t('reader.actions.label'))}">${back}
    <button class="tbtn" data-reader="reply" title="${esc(tip(t('composer.titles.reply'), 'reply'))}">${icons.reply}<span data-i18n="composer.titles.reply">${esc(t('composer.titles.reply'))}</span></button>
    <button class="icon-btn" data-reader="replyAll" title="${esc(tip(t('composer.titles.replyAll'), 'replyAll'))}">${icons.replyAll}</button>
    <button class="icon-btn" data-reader="forward" title="${esc(tip(t('composer.titles.forward'), 'forward'))}">${icons.forward}</button>
    <span class="bar-sep"></span>
    ${canArchive ? `<button class="icon-btn" data-reader="archive" title="${esc(tip(t('common.actions.archive'), 'archive'))}">${icons.archive}</button>` : ''}
    <button class="icon-btn" data-reader="delete" title="${esc(tip(t('common.actions.delete'), 'delete'))}">${icons.trash}</button>
    ${saved ? '' : `<button class="icon-btn" data-reader="move" title="${esc(tip(t('common.actions.move'), 'move'))}">${icons.move}</button>`}
    ${saved ? '' : `<button class="icon-btn" data-reader="unread" title="${esc(tip(t('mailbox.actions.unread'), readKey(false)))}">${icons.markUnread}</button>`}
    <button class="icon-btn" data-reader="more" data-i18n-title="reader.actions.more" title="${esc(t('reader.actions.more'))}" aria-haspopup="menu">${icons.more}</button>
    ${nav}</div>`;
}

function renderReader() {
  const el = $('.reader');
  if (!el || S.composer) return;
  el.classList.remove('scrolled');
  if (S.checked.size > 1) {
    const list = checkedMessages();
    const canArchive = list.some((m) => account(m.accountId)?.archive && m.role !== 'archive');
    el.innerHTML = `<div class="reader-empty multi">
      <div class="stack" aria-hidden="true"><span></span><span></span><span></span></div>
      <div class="t">${esc(t('mailbox.selection.summary', { count: list.length }))}</div>
      <div class="multi-actions">
        <button class="tbtn" data-bulk="${list.some((m) => m.unread) ? 'read' : 'unread'}">${list.some((m) => m.unread) ? icons.mailOpen : icons.markUnread}<span>${list.some((m) => m.unread) ? t('mailbox.message.read') : t('mailbox.filters.unread')}</span></button>
        <button class="tbtn" data-bulk="move">${icons.move}<span data-i18n="common.actions.move">${esc(t('common.actions.move'))}</span></button>
        ${canArchive ? `<button class="tbtn" data-bulk="archive">${icons.archive}<span data-i18n="common.actions.archive">${esc(t('common.actions.archive'))}</span></button>` : ''}
        <button class="tbtn danger" data-bulk="delete">${icons.trash}<span data-i18n="common.actions.delete">${esc(t('common.actions.delete'))}</span></button>
      </div>
      <button class="link-btn" data-bulk="clear" data-i18n="mailbox.selection.clear">${esc(t('mailbox.selection.clear'))}</button>
    </div>`;
    return;
  }
  const m = S.message;
  if (!m) {
    el.innerHTML = `<div class="reader-empty">
      ${icons.mailOpen}
      <div class="t" data-i18n="reader.empty.title">${esc(t('reader.empty.title'))}</div>
      <div class="keys"><span><kbd>${esc(shortcutLabel('Ctrl+N'))}</kbd> ${esc(t('composer.titles.new'))}</span><span><kbd>${esc(shortcutLabel('Ctrl+E'))}</kbd> ${esc(t('mailbox.search.label'))}</span><span><kbd>↑</kbd><kbd>↓</kbd> ${esc(t('reader.empty.browse'))}</span></div>
    </div>`;
    return;
  }
  const draft = m.role === 'drafts';
  const from = m.from || {};
  const body = m.error
    ? `<p class="error">${esc(m.error)}</p>`
    : S.loading || m.html === null || m.html === undefined
      ? '<div class="loading-bar"></div>'
      : `<iframe class="mail-frame" sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox" data-i18n-title="reader.content.label" title="${esc(t('reader.content.label'))}"></iframe>`;
  const unsubscribe =
    m.unsubscribe && !draft ? `<button class="chip-btn" data-reader="unsubscribe" data-i18n-title="reader.unsubscribe.tooltip" title="${esc(t('reader.unsubscribe.tooltip'))}">${icons.unsubscribe}<span data-i18n="reader.unsubscribe.action">${esc(t('reader.unsubscribe.action'))}</span></button>` : '';
  el.innerHTML = `${readerBar(m)}
    <div class="reader-scroll" tabindex="-1">
      <article class="message">
        <header class="msg-head">
          <div class="subject-row">
            <h2 class="reader-subject">${esc(m.subject || t('mailbox.message.noSubject'))}</h2>
            ${m.role === 'saved' || draft ? '' : `<button class="icon-btn reader-star ${m.starred ? 'on' : ''}" data-reader="star" title="${m.starred ? t('mailbox.actions.unstar') : t('mailbox.actions.addStar')}">${m.starred ? icons.starFilled : icons.star}</button>`}
          </div>
          <div class="msg-from">
            ${senderAvatar(draft ? (m.to || [])[0] || from : from, 'lg')}
            <div class="who">
              <div class="from-line">${
                draft
                  ? `<span class="draft-tag" data-i18n="composer.titles.draft">${esc(t('composer.titles.draft'))}</span>`
                  : `<button class="person from-name" data-person="${esc(from.address || '')}" data-name="${esc(from.name || '')}" title="${esc(from.address || '')}">${esc(person(from) || t('mailbox.message.unknownSender'))}</button>${
                      from.name ? `<span class="addr">${esc(from.address)}</span>` : ''
                    }${m.vip ? `<span class="pill vip" data-i18n="mailbox.message.vip">${esc(t('mailbox.message.vip'))}</span>` : ''}`
              }</div>
              ${recipientsHtml(m)}
            </div>
            <div class="msg-side">${unsubscribe}<time class="reader-date" title="${esc(longDate(m.date))}">${esc(readerDate(m.date))}</time></div>
          </div>
          ${detailsHtml(m)}
        </header>
        ${m.invite ? '<div class="invite-slot"></div>' : ''}
        ${attachmentsHtml(m)}
        <div class="related-slot">${relatedHtml(m)}</div>
        ${body}
      </article>
    </div>`;
  renderReaderNav();
  const scroll = el.querySelector('.reader-scroll');
  scroll.addEventListener('scroll', () => el.classList.toggle('scrolled', scroll.scrollTop > 56), { passive: true });
  const frame = el.querySelector('.mail-frame');
  if (frame && m.html !== null && m.html !== undefined) {
    const s = S.data?.settings || {};
    fillFrame(frame, m, { light: isLight(), darkEmails: s.darkEmails !== false, fitContent: s.fitContent !== false });
  }
  // A partly shown message gets its thumbnails once the whole message is in (completeMessage).
  if (!S.loading && !m.partial) loadPreviews(m);
  const slot = el.querySelector('.invite-slot');
  if (slot) mountInvite(slot, { id: m.id }, m.invite, { openEvent: openCalendarEvent }).catch(() => {});
  wantLogos([m]);
}

async function unsubscribe(m) {
  const sender = person(m.from) || t('reader.unsubscribe.unknownSender');
  const ok = await confirmDialog(
    t('reader.unsubscribe.title', { sender: sender }),
    m.unsubscribe.oneClick || m.unsubscribe.url
      ? t('reader.unsubscribe.linkHelp')
      : t('reader.unsubscribe.mailHelp'),
    t('reader.unsubscribe.action')
  );
  if (!ok) return;
  try {
    const res = await api('unsubscribe', m.id);
    if (res.done) toast(t('reader.unsubscribe.done', { sender: sender }));
    else if (res.opened) toast(t('reader.unsubscribe.opened'));
    else if (res.message) {
      const m = res.message;
      compose({ mode: 'new', to: m.to, cc: m.cc, bcc: m.bcc, subject: m.subject || t('reader.unsubscribe.subject'), body: m.body });
    }
  } catch (err) {
    toast(err.message, 5000);
  }
}

function personMenu(btn) {
  const address = btn.dataset.person;
  if (!address) return;
  const name = btn.dataset.name;
  const vip = (S.data.settings.vips || []).includes(address.toLowerCase());
  showMenu(
    btn,
    [
      { heading: name ? `${name} <${address}>` : address },
      { icon: 'compose', get label() { return t('composer.titles.new'); }, action: () => compose({ mode: 'new', to: [{ name, address }] }) },
      {
        icon: 'link',
        get label() { return t('reader.address.copy'); },
        action: async () => {
          await navigator.clipboard.writeText(address).catch(() => {});
          toast(t('reader.address.copied'));
        }
      },
      {
        icon: 'vip',
        label: vip ? t('mailbox.vip.remove') : t('mailbox.vip.add'),
        action: async () => {
          const on = await api('toggleVip', address);
          toast(on ? t('mailbox.vip.added') : t('mailbox.vip.removed'));
        }
      }
    ],
    { align: 'left' }
  );
}

function bindReader() {
  $('.reader').addEventListener('click', async (e) => {
    const btn = e.target.closest('button');
    if (!btn) return;
    if (btn.dataset.bulk) return bulkAction(btn.dataset.bulk, btn);
    const m = S.message;
    if (!m) return;
    if (btn.dataset.related) {
      const known = S.related && S.related.id === m.id ? S.related.list.find((r) => r.id === btn.dataset.related) : null;
      // Like a click on a row: a checked email would otherwise stay the target of Delete and the other actions.
      clearChecks();
      return openMessage(btn.dataset.related, { known });
    }
    const item = byId(m.id) || m;
    if (btn.dataset.person !== undefined) return personMenu(btn);
    if (btn.dataset.openAtt) {
      const index = Number(btn.dataset.openAtt);
      return wholeMessage(m, index)
        .then(() => api('openAttachment', m.id, index))
        .catch((err) => toast(err.message));
    }
    if (btn.dataset.saveAtt) {
      const index = Number(btn.dataset.saveAtt);
      const p = await wholeMessage(m, index)
        .then(() => api('saveAttachment', m.id, index))
        .catch((err) => toast(err.message));
      if (p) toast(t('reader.attachments.savedOne'));
      return;
    }
    switch (btn.dataset.reader) {
      case 'back':
        return closeReader();
      case 'expand':
        S.expanded = !S.expanded;
        renderShell();
        return renderReader();
      case 'prev':
      case 'next':
        return step(btn.dataset.reader === 'next' ? 1 : -1);
      case 'star':
        return setStarred([item], !item.starred);
      case 'unread':
        setUnread([item], true);
        toast(t('mailbox.actions.markedUnread'));
        return;
      case 'details':
        S.details = !S.details;
        return renderReader();
      case 'unsubscribe':
        return unsubscribe(m);
      case 'toggle-atts':
        S.attachmentsOpen = !S.attachmentsOpen;
        return renderReader();
      // In place, so the email under it doesn't redraw; the list folds away with its header.
      case 'toggle-related': {
        prefs.relatedOpen = prefs.relatedOpen === false;
        savePrefs();
        const section = btn.closest('.related');
        section.classList.toggle('closed', !prefs.relatedOpen);
        section.querySelector('.rel-body').inert = !prefs.relatedOpen;
        btn.setAttribute('aria-expanded', String(prefs.relatedOpen));
        return;
      }
      case 'related-all': {
        S.relatedAll = m.id;
        const slot = $('.reader .related-slot');
        slot.innerHTML = relatedHtml(m);
        // The keyboard goes on to the first row that just appeared.
        slot.querySelectorAll('.rel')[RELATED_SHOWN]?.focus();
        return;
      }
      case 'save-all': {
        const n = await wholeMessage(m, 'all')
          .then(() => api('saveAllAttachments', m.id))
          .catch((err) => toast(err.message, 5000));
        if (n) toast(t('reader.attachments.savedCount', { count: n }));
        return;
      }
      case 'edit':
        return compose({ mode: 'draft', id: m.id });
      case 'reply':
      case 'replyAll':
      case 'forward':
        if (m.error) return;
        return compose({ mode: btn.dataset.reader, id: m.id });
      case 'archive':
        return archiveMessages([item]);
      case 'move':
        return moveWithPicker([item]);
      case 'delete':
        return removeMessages([item]);
      case 'more':
        return showMenu(btn, messageMenu(item));
    }
  });
}

function step(dir, extend = false) {
  const list = visibleRows();
  if (!list.length) return;
  const from = cursor();
  const i = list.findIndex((x) => x.id === from);
  const target = list[i < 0 ? 0 : Math.max(0, Math.min(list.length - 1, i + dir))];
  if (!target) return;
  if (extend) {
    // Grow or shrink the selection from its anchor; the message on screen does not change.
    if (!S.checked.size && S.selectedId) S.checked.add(S.selectedId);
    const anchor = list.some((x) => x.id === S.anchorId) ? S.anchorId : from || target.id;
    S.anchorId = anchor;
    const a = list.findIndex((x) => x.id === anchor);
    const b = list.findIndex((x) => x.id === target.id);
    S.checked = new Set(list.slice(Math.min(a, b), Math.max(a, b) + 1).map((m) => m.id));
    S.cursorId = target.id;
    selectionChanged();
    revealRow(target.id);
    return;
  }
  if (S.checked.size) {
    S.checked.clear();
    selectionChanged();
  }
  if (target.id === S.selectedId) {
    S.cursorId = target.id;
    return syncRowClasses();
  }
  openMessage(target.id);
}

// ---------- divider-free layout changes ----------

window.addEventListener('resize', () => {
  if (S.data) renderShell();
});

// ---------- keyboard ----------

function editing(e) {
  const t = e.target;
  return t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
}

document.addEventListener('keydown', (e) => {
  if (menuOpen()) return;
  if (document.querySelector('.page, .scrim')) return;
  const ctrl = e.ctrlKey || e.metaKey;
  const key = e.key.toLowerCase();
  // Ctrl+J works everywhere, the editor included: it opens the chat and puts you in its input,
  // or closes it again from an empty input. Without it, J would also step to the next message.
  if (ctrl && key === 'j' && !e.shiftKey && !e.altKey && S.data && S.data.accounts.length) {
    e.preventDefault();
    // While the agent is not set up the input is hidden, so there is nothing to focus: Ctrl+J just closes.
    if (prefs.agentOpen && agentPanel && (agentPanel.inputFocused() || !agentPanel.inputAvailable())) {
      if (!agentPanel.inputAvailable() || !agentPanel.inputValue().trim()) setAgentOpen(false);
      return;
    }
    return setAgentOpen(true, { focus: true });
  }
  if (e.target.closest && e.target.closest('.composer')) return;
  if (!S.data || !S.data.accounts.length) return;
  // Ctrl+1 and Ctrl+2 switch between Mail and Calendar, as in Outlook; Ctrl+3 opens the meetings.
  if (ctrl && !e.shiftKey && !e.altKey && (e.key === '1' || e.key === '2' || e.key === '3')) {
    e.preventDefault();
    return setMode(e.key === '1' ? 'mail' : e.key === '2' ? 'calendar' : 'meeting');
  }
  if (calendarMode()) {
    if (!e.target.closest?.('.agentpane, .bui-menu, .sort-panel') && calendar.onKey(e)) e.preventDefault();
    return;
  }
  if (meetingMode()) {
    if (!e.target.closest?.('.agentpane, .bui-menu') && meeting.onKey(e)) e.preventDefault();
    return;
  }
  // The rest follows the chosen keyboard scheme (keymap.js). Writing, searching and syncing work while typing too,
  // as long as their shortcut has a modifier; single keys never act in a text field.
  const typing = editing(e);
  let action = actionFor(e, { typing });
  // Shift with a step key extends the selection.
  if (!action && e.shiftKey) {
    const plain = actionFor({ key: e.key, ctrlKey: e.ctrlKey, metaKey: e.metaKey, altKey: e.altKey, shiftKey: false }, { typing });
    if (plain === 'next' || plain === 'previous') action = plain;
  }
  if (action === 'compose' || action === 'search' || action === 'sync') {
    e.preventDefault();
    return action === 'compose' ? compose({ mode: 'new' }) : action === 'search' ? focusSearch() : syncNow();
  }
  // Keys in the chat panel stay there: Delete on one of its buttons must not delete the open email.
  // Its menus (agent picker, history) live on the body, outside the panel. The same goes for the sorting panel.
  if (e.target.closest?.('.agentpane, .bui-menu, .sort-panel')) return;
  if (typing) return;
  if (ctrl && key === 'z' && !e.shiftKey) {
    if (runToastAction()) e.preventDefault();
    return;
  }
  // Before Esc's own meaning: Shift+Esc marks the list read and leaves the selection and the editor as they are.
  if (action === 'readAll') {
    e.preventDefault();
    return markAllRead();
  }
  const list = targets();
  const m = S.composer ? null : S.message;
  if (e.key === 'Escape') {
    if (clearChecks()) return;
    if (S.composer) return S.composer.close();
    if (S.expanded) {
      S.expanded = false;
      renderShell();
      return renderReader();
    }
    if (S.query) return clearSearch();
    return;
  }
  if (e.key === 'Enter' && m && m.role === 'drafts' && !e.target.closest?.('button')) {
    e.preventDefault();
    return compose({ mode: 'draft', id: m.id });
  }
  if (action && shortcutAction(action, list, m, e) !== false) {
    e.preventDefault();
    return;
  }
  if ((e.key === 'ArrowRight' || e.key === 'ArrowLeft') && !ctrl && !e.altKey && !e.shiftKey && stackArrow(e)) {
    e.preventDefault();
    return;
  }
  if ((e.key === 'Home' || e.key === 'End') && e.target.closest?.('.list-scroll')) {
    e.preventDefault();
    const all = visibleRows();
    const target = e.key === 'Home' ? all[0] : all[all.length - 1];
    if (target) openMessage(target.id);
  }
});

// What a shortcut does with the selected emails (list) or the open one (m); false when it doesn't apply here.
function shortcutAction(action, list, m, e) {
  const mail = m && m.role !== 'drafts';
  switch (action) {
    case 'reply':
    case 'replyAll':
    case 'forward':
      return mail ? compose({ mode: action, id: m.id }) : false;
    case 'print':
      return m ? printMessage(m.id) : false;
    case 'selectAll':
      S.checked = new Set(rows().map((x) => x.id));
      return selectionChanged();
    case 'next':
    case 'previous':
      // Arrow keys in the open email scroll it.
      if (e.target.closest?.('.reader-scroll, .divider')) return false;
      $('.list-scroll')?.classList.add('kbd');
      if (document.activeElement === document.body) $('.list-scroll')?.focus();
      return step(action === 'next' ? 1 : -1, e.shiftKey);
  }
  if (!list.length) return false;
  switch (action) {
    case 'archive': {
      const can = list.filter((x) => canArchive(x));
      return can.length ? archiveMessages(can) : false;
    }
    case 'delete':
      return removeMessages(list);
    case 'read':
      return setUnread(list, false);
    case 'unread':
      return setUnread(list, true);
    case 'toggleRead':
      return setUnread(list, !list.some((x) => x.unread));
    case 'star':
      return setStarred(list, !list.every((x) => x.starred));
    case 'move':
      return moveWithPicker(list);
    case 'category': {
      const sorted = list.filter((x) => x.category);
      return sorted.length ? moveToCategory(sorted) : false;
    }
  }
  return false;
}

// ---------- taskbar badge ----------

function drawBadge(count) {
  if (!count) return api('setBadge', null, 0);
  const c = document.createElement('canvas');
  c.width = c.height = 32;
  const g = c.getContext('2d');
  g.fillStyle = '#d6336c';
  g.beginPath();
  g.arc(16, 16, 16, 0, Math.PI * 2);
  g.fill();
  g.fillStyle = '#fff';
  const label = count > 99 ? '99+' : String(count);
  g.font = `bold ${label.length > 2 ? 13 : label.length > 1 ? 17 : 20}px Segoe UI`;
  g.textAlign = 'center';
  g.textBaseline = 'middle';
  g.fillText(label, 16, 17);
  return api('setBadge', c.toDataURL(), count);
}

// ---------- new mail ----------

// New mail main told this window about (main.js showMailNotices) whose rows are in the list on screen, until one of
// them has been in view. While the list is scrolled away from all of them, a pill at the top counts them.
const newRows = new Set();
// The emails the new-mail toast counts, and when it showed, so mail from the next account in the same sync adds to it.
let newToast = { mails: [], at: 0 };
const NEW_TOAST_MS = 8000;
let coo = null;

// Mail mode, with no Settings page over the list, and the list not behind the reader in a narrow window.
function listOnScreen() {
  const scroll = $('.list-scroll');
  return prefs.mode === 'mail' && !$('#overlays .page') && Boolean(scroll && scroll.getClientRects().length);
}

function updateNewPill() {
  const pill = $('.new-pill');
  if (!pill) return;
  if (newRows.size) {
    const shown = new Set(visibleRows().map((m) => m.id));
    for (const id of newRows) if (!shown.has(id)) newRows.delete(id);
  }
  const scroll = $('.list-scroll');
  const els = newRows.size && listOnScreen() ? $$('.list-scroll .item').filter((el) => newRows.has(el.dataset.id)) : [];
  if (!els.length) {
    pill.hidden = true;
    return;
  }
  const box = scroll.getBoundingClientRect();
  const rects = els.map((el) => el.getBoundingClientRect());
  if (rects.some((r) => r.bottom > box.top && r.top < box.bottom)) {
    newRows.clear();
    pill.hidden = true;
    return;
  }
  const up = rects[0].bottom <= box.top;
  drawHtml(pill, `${up ? icons.up : icons.down}<span>${esc(t('mailbox.newMail.count', { count: els.length }))}</span>`);
  pill.style.top = `${scroll.offsetTop + 8}px`;
  pill.hidden = false;
}

// The pill's click: the first new row comes into view, below the day heading that sticks to the top.
function showNewRows() {
  const scroll = $('.list-scroll');
  const el = $$('.list-scroll .item').find((x) => newRows.has(x.dataset.id));
  if (!el) return updateNewPill();
  const gap = ($('.list-scroll .group-head')?.offsetHeight || 0) + 4;
  const top = scroll.scrollTop + el.getBoundingClientRect().top - scroll.getBoundingClientRect().top - gap;
  scroll.focus({ preventScroll: true });
  scroll.scrollTo({ top: Math.max(0, top), behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
}

// A desktop notification's click, or the new-mail toast's Open: the email in Inbox.
async function openNotified(id) {
  setMode('mail');
  S.scope = 'all';
  S.view = 'inbox';
  S.folder = null;
  S.category = null;
  S.query = '';
  S.filter = 'all';
  await refresh();
  openMessage(id);
}

// The new-mail toast's Show: Inbox as from the sidebar, or the category tab that holds the emails when Inbox leaves
// them out (newMailTab).
function showNotified(tab) {
  setMode('mail');
  S.scope = 'all';
  S.view = 'inbox';
  S.folder = null;
  S.category = categoryOf(tab) ? tab : null;
  changeView();
}

// While the window is in front, a toast stands in for the desktop notification for mail the list on screen doesn't
// show. It waits for an Undo that is up, and Ctrl+Z leaves it alone. Over Settings it only says so.
// mails: [{ id, from, subject, category }].
function newMailToast(mails) {
  const now = Date.now();
  const all = addToToast(now - newToast.at < NEW_TOAST_MS ? newToast.mails : [], mails);
  newToast = { mails: all, at: now };
  // Several emails go to Inbox, or to their category when Inbox doesn't show it.
  const { message, open, tab } = newMailToastFor(all, S.categories, t);
  if ($('#overlays .page')) return toast(message, { wait: true, ms: 4000 });
  // Once its button was used, the next new mail gets a toast of its own.
  const run = () => {
    newToast = { mails: [], at: 0 };
    return open ? openNotified(open) : showNotified(tab);
  };
  toast(message, { wait: true, action: { label: t(open ? 'mailbox.newMail.open' : 'mailbox.newMail.show'), shortcut: false, run } });
}

// Settings > General's sound for new mail: the coo from rukoo.io, quietly. Main says when (newMailSound()).
function playCoo() {
  coo ||= Object.assign(new Audio('assets/coo.mp3'), { volume: 0.5 });
  coo.currentTime = 0;
  coo.play().catch(() => {});
}

// p: { mails: [{ id, from, subject, category }], toast, sound }. Rows in the list on screen are the cue themselves, or
// the pill points at them; for the rest of the batch a toast says so, when main says the window is in front with
// notifications on. One sync can bring both.
async function onNewMail(p) {
  const mails = p && Array.isArray(p.mails) ? p.mails.filter((m) => m && m.id) : [];
  if (!mails.length) return;
  if (p.sound) playCoo();
  if (!S.data || !S.data.accounts.length) return;
  await refresh();
  const { here, elsewhere } = splitNewMail(mails, listOnScreen() ? new Set(rows().map((m) => m.id)) : new Set());
  if (here.length) {
    for (const m of here) newRows.add(m.id);
    updateNewPill();
  }
  if (elsewhere.length && p.toast) newMailToast(elsewhere);
}

// ---------- events from main ----------

window.mail.on(async ({ type, payload }) => {
  if (type === 'updated') scheduleRefresh();
  if (type === 'calendar') calendar.reload();
  // Rukoo Voice and its meetings (main.js voice block).
  if (type === 'meeting') meeting.onEvent(payload);
  // A reminder or a booking notification was clicked: the calendar opens on that event or request, or on the day
  // when the notification names only its start.
  if (type === 'calendar-open' && payload && S.data && S.data.accounts.length) {
    setMode('calendar');
    if (typeof payload.request === 'string') calendar.openRequest(payload.request);
    else if (typeof payload.id === 'string') calendar.openEvent(payload.id, Number.isFinite(payload.start) ? payload.start : undefined);
    else if (Number.isFinite(payload.start)) calendar.openDay(payload.start);
  }
  // The booking page's state: requests and booked events on the grid, its notices above it, the link for the composer.
  // Its notices come translated from main, so a language change asks for them again.
  if (type === 'booking') applyBooking(payload);
  // An .ics file opened with Rukoo, or an .ics attachment: its card.
  if (type === 'invite-file') takeInviteFiles();
  if (type === 'language') api('bookingState').then(applyBooking).catch(() => {});
  if (type === 'categories') {
    S.categories = payload;
    setCategories(payload);
    renderSidebar();
    renderList();
  }
  if (type === 'categories-progress') setActivity(payload);
  if (type === 'updates') {
    S.updates = payload;
    renderSidebar();
  }
  if (type === 'language') await refresh();
  if (type === 'badge') drawBadge(payload);
  if (type === 'toast') toast(payload);
  // Undo send (main.js): an email waits after Send, Don't quit kept emails waiting, or the server refused one that
  // waited and its editor opens again.
  if (type === 'send-queued' && payload && typeof payload.id === 'string') offerUndoSend(payload);
  if (type === 'send-done' && payload) waitedMailSent(payload);
  if (type === 'send-paused') takeUndoNotices();
  if (type === 'send-failed') takeSendFailures();
  // A paired phone fetched the accounts and settings: said wherever the user is, since it sends secrets.
  if (type === 'agent' && payload && payload.kind === 'phone' && payload.event === 'exported') {
    toast(t('settings.phone.exported', { name: (payload.pairing && payload.pairing.name) || '' }), 6000);
  }
  if (type === 'theme') {
    applyTheme(S.data?.settings.theme);
    if (S.message) renderReader();
    S.composer?.retheme(S.data);
  }
  if (type === 'open-message') await openNotified(payload);
  if (type === 'new-mail') await onNewMail(payload);
});

onSystemThemeChange(() => {
  applyTheme(S.data?.settings.theme);
  if (S.message) renderReader();
  S.composer?.retheme(S.data);
});

// Closing the window with unsaved text in the editor: keep it open and ask first. Main hears the outcome, because
// a quit waits for it. A busy editor answers without asking, or with the question that is already open.
window.addEventListener('beforeunload', (e) => {
  const editor = S.composer;
  if (!editor || (!editor.isDirty() && !editor.isBusy())) return;
  e.preventDefault();
  e.returnValue = false;
  setTimeout(() => editor.close().then((done) => api('unloadAnswer', done)).catch(() => {}), 0);
});

// Scrollbars show while scrolling and fade after.
const scrolling = new Map();
document.addEventListener(
  'scroll',
  (e) => {
    const el = e.target;
    if (!(el instanceof Element)) return;
    el.classList.add('is-scrolling');
    clearTimeout(scrolling.get(el));
    scrolling.set(el, setTimeout(() => el.classList.remove('is-scrolling'), 900));
  },
  true
);

// "What's new": after "Sort my inbox" the inbox shows its categories, and with Jev on, what Jev is doing.
function showWhatsNew() {
  return openWhatsNew().then(async (done) => {
    if (!done) return done;
    await refresh();
    if (S.categories && S.categories.enabled && S.categories.jev.on && !sortPanelOpen()) toggleSortPanel();
    return done;
  });
}

// An email from Jev's last decisions: the inbox, in the category it went to, with the email open.
async function openSorted(id, category) {
  const accountId = decodeURIComponent(String(id).split(':')[0]);
  if (S.scope !== 'all' && S.scope !== accountId) S.scope = S.data.accounts.length > 1 ? 'all' : accountId;
  if (prefs.mode !== 'mail') setMode('mail');
  S.view = 'inbox';
  S.folder = null;
  S.category = S.categories && S.categories.categories.some((c) => c.id === category) ? category : null;
  S.query = '';
  S.filter = 'all';
  S.checked.clear();
  await refresh();
  if (S.list.some((m) => m.id === id)) openMessage(id);
}

initSortActivity({ settings: () => openSettings(ctx, { view: 'categories' }), open: (id, category) => openSorted(id, category) });

// Context shared with the settings and setup pages and the chat panel.
export const ctx = {
  S,
  refresh,
  account,
  openMessage,
  closeReader,
  compose,
  // An email in the list, or the open one; the chat panel adds dragged emails by id.
  messageById: (id) => byId(id),
  openSetup: (opts) => openSetup(ctx, opts),
  openWhatsNew: () => showWhatsNew(),
  // view: open straight on one page, such as 'agents', or an account's 'server' page with { id }.
  openSettings: (view, opts = {}) => openSettings(ctx, { view, ...opts }),
  renderSidebar: () => renderSidebar(),
  revealAgent: () => setAgentOpen(true),
  closeAgent: () => setAgentOpen(false),
  agentOpen: () => Boolean(prefs.agentOpen),
  agentOverlay: () => Boolean(prefs.agentOpen) && OVERLAY_QUERY.matches,
  agentNews: (tone) => setAgentNews(tone),
  // For settings that change how mail is drawn: the open mail and the editor's quote.
  redrawMail: () => {
    if (S.message) renderReader();
    S.composer?.retheme(S.data);
  },
  // Rukoo Voice's meetings (meeting.js): the chat panel follows the one on screen in meeting mode, and a calendar
  // card records an event.
  meetingMode: () => meetingMode(),
  currentMeeting: () => (meetingMode() ? meeting.current() : null),
  openMeeting: (id) => openMeeting(id),
  voiceState: () => meeting.voiceState(),
  recordMeeting: (opts) => meeting.record(opts),
  // The arrow beside a card's Record: what Voice records of the screen. opts: {onPick(screen)} or {title, eventId}.
  recordMeetingMenu: (anchor, opts) => meeting.recordMenu(anchor, opts),
  openCalendarEvent: (id, start) => openCalendarEvent(id, start),
  agentSync: () => agentViewChanged()
};

calendar.init(ctx);
meeting.init(ctx);
api('bookingState').then(applyBooking).catch(() => {});
refresh().catch((err) => toast(err.message, 6000));
