import { t, getLocale } from './i18n.js';
import { icons } from './icons.js';
import { api, esc, $, hhmm, hue, initials, readerDate, groupLabel, toast, showMenu, closeMenu, menuOpen, menuCount, segmented, setSegmented, confirmDialog } from './ui.js';
import { formatButtons, mountRichEditor } from './rich-editor.js';
import { markdownToNotes, notesToMarkdown } from './notes-markdown.js';
import { foldFrames } from './meeting-frames.js';
import { editMenu } from './editmenu.js';
import { promptDialog } from './settings.js';

// The meeting view: Rukoo Voice's meetings, a third mode next to Mail and Calendar (Ctrl+3). Voice records and
// transcribes; main keeps what Rukoo owns of each meeting (src/main/voice) and sends 'meeting' events. This module
// draws the meeting on screen with its controls, the transcript as it comes in with the screen's frames at their
// moments, who spoke, the linked calendar event and the user's notes. It also draws the list of meetings in the
// sidebar and the recording indicator in the title bar.
//
// Everything in a meeting was said, shown or written by other people, and attendee names come from whoever sent the
// invitation; text read on the screen and monitor names come from the screen: it reaches the DOM only through esc()
// or textContent.

const SAVED = 'rukoo.meeting';
const NOTES_WAIT = 700;
const STOP_CONFIRM_MS = 3000;
// Two passages of one voice this close together read as one stretch: the name shows once.
const SAME_STRETCH_MS = 30000;
const LIVE = new Set(['recording', 'paused']);
const VOICE_ERRORS = new Set(['offline', 'timeout', 'busy', 'not_found', 'not_recording', 'not_available', 'failed', 'bad_request', 'unknown_command']);
// What Voice records of the screen in a meeting (voice/PROTOCOL.md, meeting.started).
const SCREENS = ['off', 'frames', 'video'];
const SCREEN_MODES = { off: 'meeting.screen.modeOff', frames: 'meeting.screen.modeFrames', video: 'meeting.screen.modeVideo' };
// Frame images kept in memory for the meeting on screen; the least recently used goes first.
const FRAME_CACHE = 60;
// A frame's image starts loading when its row comes this close to the visible part of the transcript.
const FRAME_MARGIN_PX = 300;
const FRAME_MARGIN = `${FRAME_MARGIN_PX}px 0px`;

const M = {
  open: false,
  // What main says about Rukoo Voice (voiceState in main.js). screenDefault: the screen a meeting records when
  // Rukoo doesn't choose, from Voice's Settings, or null when Voice didn't say.
  voice: { status: 'absent', reason: 'none', voiceVersion: null, older: null, recording: null, dictating: false, hidden: false, canHide: false, screenDefault: null },
  // Summaries, newest first (meetingList).
  list: [],
  listLoaded: false,
  // The meeting on screen and everything about it (meetingGet), or null while it loads or when Rukoo lacks it.
  id: readSaved(),
  data: null,
  // The meeting the last meetingGet answered for: until then the view shows that it loads.
  fetched: null,
  loading: 0,
  // In a narrow window the transcript, the notes and the people share the view as tabs.
  tab: 'transcript',
  starting: false,
  confirmStop: null,
  renaming: null,
  renameBusy: false,
  deleting: new Set(),
  deleted: new Set(),
  // The passage whose text the user is correcting; changes to it wait until the editor closes.
  editing: null,
  // The Voice that answered screen.get with unknown_command: its meetings show no Screen button.
  noScreenSwitch: null,
  monitors: new Map(),
  // Frames the user opened: a large image and all of their text.
  frameOpen: new Set(),
  groupOpen: new Set(),
  showFrames: localStorage.getItem('rukoo.meeting.showFrames') !== '0'
};
let ctx = null;
let bound = false;

function readSaved() {
  try {
    const id = localStorage.getItem(SAVED);
    return typeof id === 'string' && /^m-[0-9]{8}-[0-9]{6}-[0-9a-f]{4}$/.test(id) ? id : null;
  } catch (_) {
    return null;
  }
}
function saveId(id) {
  try {
    if (id) localStorage.setItem(SAVED, id);
    else localStorage.removeItem(SAVED);
  } catch (_) {
    // A full or blocked storage only means the view opens on the newest meeting next time.
  }
}

// ---------- formatting ----------

const pad = (n) => String(n).padStart(2, '0');
// Meeting time: "4:05", or "1:04:05" past the hour.
function clock(ms) {
  const s = Math.max(0, Math.floor((Number(ms) || 0) / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}
// A stretch of time in words: "12 sec", "1 min, 5 sec".
function length(ms) {
  const total = Math.max(1, Math.round((Number(ms) || 0) / 1000));
  const parts = { hours: Math.floor(total / 3600), minutes: Math.floor((total % 3600) / 60), seconds: total % 60 };
  if (parts.hours) delete parts.seconds;
  for (const k of Object.keys(parts)) if (!parts[k]) delete parts[k];
  try {
    return new Intl.DurationFormat(getLocale(), { style: 'short' }).format(parts);
  } catch (_) {
    return [parts.hours && `${parts.hours} h`, parts.minutes && `${parts.minutes} min`, parts.seconds && `${parts.seconds} s`].filter(Boolean).join(' ');
  }
}
const live = (s) => Boolean(s) && LIVE.has(s.state);
// How long the meeting has run: up to now while it records or pauses (the clock keeps running), else its length.
function elapsed(s, now = Date.now()) {
  if (!s || !Number.isFinite(s.startedAt)) return null;
  const end = live(s) ? now : s.endedAt;
  return Number.isFinite(end) ? Math.max(0, end - s.startedAt) : null;
}
const summaryOf = (id) => M.list.find((m) => m.id === id) || null;
// A linked event's time, short enough for the side column: "Mon 2 Nov, 09:30 – 09:45".
function eventWhen(e) {
  if (!Number.isFinite(e.start) || !Number.isFinite(e.end)) return '';
  const day = new Intl.DateTimeFormat(getLocale(), { weekday: 'short', day: 'numeric', month: 'short' }).format(new Date(e.start));
  return `${day}, ${hhmm(e.start)} – ${hhmm(e.end)}`;
}
// The meeting's own title, else the linked event's, else when it started.
function titleOf(s, event = null) {
  if (s && s.title && s.title.trim()) return s.title.trim();
  if (event && !event.missing && event.title) return event.title;
  if (s && Number.isFinite(s.startedAt)) return t('meeting.untitled', { time: hhmm(s.startedAt) });
  return t('meeting.untitledNoTime');
}
// A screen mode in words, inside a sentence: "frames".
const screenMode = (mode) => t(SCREEN_MODES[mode] || SCREEN_MODES.off);
// What a meeting records of the screen, or null for nothing (also for a value this Rukoo doesn't know).
const screenOf = (s) => (s && SCREENS.includes(s.screen) && s.screen !== 'off' ? s.screen : null);

// What Voice said back when a command failed, in the user's words.
export function voiceError(err) {
  const code = err && err.voice;
  return VOICE_ERRORS.has(code) ? t(`meeting.errors.${code}`) : (err && err.message) || t('meeting.errors.failed');
}

// Why Rukoo can't record now, or null when it can.
function notReadyText(v = M.voice) {
  if (v.status === 'connected') return v.recording ? t('meeting.errors.busy') : null;
  if (v.status === 'connecting') return t('meeting.voice.connecting');
  if (v.status === 'incompatible') return v.older === 'rukoo' ? t('meeting.voice.updateRukoo') : t('meeting.voice.updateVoice');
  if (v.status === 'insecure') return t('meeting.voice.insecure');
  return t('meeting.voice.absent');
}

// ---------- the API app.js uses ----------

export function init(context) {
  ctx = context;
  document.addEventListener('contextmenu', (e) => {
    const row = e.target.closest('[data-mt-open]');
    if (!row) return;
    e.preventDefault();
    meetingMenu(row, row.dataset.mtOpen, { at: { x: e.clientX, y: e.clientY } });
  });
  document.addEventListener('keydown', (e) => {
    const row = e.target.closest('[data-mt-open]');
    if (row && (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey))) {
      e.preventDefault();
      meetingMenu(row, row.dataset.mtOpen);
    }
  });
  Promise.all([api('voiceState').catch(() => null), api('meetingList').catch(() => null)]).then(([state, list]) => {
    if (state) M.voice = state;
    if (Array.isArray(list)) M.list = list;
    M.listLoaded = true;
    afterVoice();
    redrawSide();
    if (M.open) pickAndOpen();
  });
}

export const isOpen = () => M.open;
export const voiceState = () => ({ ...M.voice });

export function show() {
  M.open = true;
  bind();
  pickAndOpen();
}

export function hide() {
  M.open = false;
  flushNotes();
  closeEditor(false);
}

// After a language change everything is drawn again.
export function relocalize() {
  const el = $('.meeting');
  // The notes field is built again: what was typed goes out first, and stays in the new field.
  const area = $('.meeting .mt-notes-input');
  const typed = area && (N.dirty || N.saving) ? notesToMarkdown(area) : null;
  flushNotes();
  if (el) {
    el.innerHTML = '';
    delete el.dataset.id;
  }
  renderIndicator();
  if (M.open) renderView();
  const next = $('.meeting .mt-notes-input');
  if (next && typed !== null) next.innerHTML = markdownToNotes(typed);
}

// The meeting the chat panel follows: the one on screen in meeting mode.
export function current() {
  if (!M.open || !M.id) return null;
  const s = (M.data && M.data.meeting) || summaryOf(M.id);
  if (!s || s.removed) return null;
  return { id: M.id, title: titleOf(s, M.data && M.data.event), eventId: s.eventId || null };
}

// Opens one meeting in the view (app.js has switched to meeting mode).
export function open(id) {
  if (!id) return;
  if (id === M.id && M.data) {
    renderView();
    return;
  }
  flushNotes();
  M.renaming = null;
  closeEditor(false);
  M.id = id;
  M.data = null;
  M.fetched = null;
  M.tab = 'transcript';
  M.frameOpen.clear();
  M.groupOpen.clear();
  forgetFrames(id);
  saveId(id);
  renderView();
  redrawSide();
  const fetched = fetchMeeting(id);
  ctx.agentSync?.();
  return fetched;
}

// Starts a recording through Rukoo Voice: from the sidebar, the empty view or a calendar event's card. opts:
// {title, eventId, screen}; without screen Voice records with its own default. The new meeting opens once Voice
// has it.
export async function record(opts = {}) {
  const why = notReadyText();
  if (why) return toast(why, 5000);
  if (M.starting) return;
  M.starting = true;
  renderSideSoon();
  try {
    const screen = SCREENS.includes(opts.screen) ? { screen: opts.screen } : {};
    const res = await api('meetingStart', { title: opts.title || '', eventId: opts.eventId || null, ...screen });
    if (res && res.meetingId) {
      await reloadList();
      ctx.openMeeting(res.meetingId);
    }
  } catch (err) {
    toast(voiceError(err), 6000);
  } finally {
    M.starting = false;
    renderSideSoon();
  }
}

// What Voice records of the screen in the next meeting: Voice's own default, audio only, frames or video. onPick
// gets the screen to pass to record(), or undefined for Voice's default.
export function screenMenu(anchor, onPick, { align = 'right', at = null } = {}) {
  const def = SCREENS.includes(M.voice.screenDefault) ? M.voice.screenDefault : null;
  showMenu(
    anchor,
    [
      { heading: t('meeting.screen.choose') },
      { label: def ? t('meeting.screen.default', { mode: screenMode(def) }) : t('meeting.screen.defaultPlain'), checked: true, action: () => onPick(undefined) },
      { label: t('meeting.screen.off'), checked: false, action: () => onPick('off') },
      { label: t('meeting.screen.frames'), checked: false, action: () => onPick('frames') },
      { label: t('meeting.screen.video'), checked: false, action: () => onPick('video') }
    ],
    { align, at }
  );
}

// The arrow beside a Record button. opts: {title, eventId} for the recording, or onPick to start it yourself, as
// the calendar's card does after closing.
export function recordMenu(anchor, opts = {}) {
  const why = notReadyText();
  if (why) return toast(why, 5000);
  if (M.starting) return;
  const pick = typeof opts.onPick === 'function' ? opts.onPick : (screen) => record({ title: opts.title, eventId: opts.eventId, screen });
  // In the sidebar the menu hangs under the whole button, not only its arrow.
  const split = anchor.closest('.mt-record-split.is-side');
  const r = split && split.getBoundingClientRect();
  screenMenu(anchor, pick, r ? { at: { x: r.left, y: r.bottom + 4 } } : {});
}

// The meeting view takes its keys itself; app.js passes on what reaches the document.
export function onKey() {
  return false;
}

// Main's 'meeting' events (main.js voice block): the Voice state, a change to one meeting, a removed meeting, or
// "fetch the list again".
export function onEvent(p) {
  if (!p || typeof p !== 'object') return;
  if (p.kind === 'state' && p.state) return setVoice(p.state);
  if (p.kind === 'list') return reloadList().then(() => redrawSide());
  // Voice deleted a meeting. Main keeps it only for the notes the user wrote, marked removed.
  if (p.kind === 'removed') {
    if (p.purge) {
      M.deleted.add(p.id);
      N.queued.delete(p.id);
      N.blocked.delete(p.id);
      if (N.id === p.id) {
        clearTimeout(N.timer);
        N.dirty = false;
      }
    }
    M.list = M.list.filter((m) => m.id !== p.id);
    if (p.id === M.id) {
      M.data = null;
      M.fetched = null;
    }
    return reloadList().then(() => {
      if (p.id === M.id && M.open) fetchMeeting(M.id);
      redrawSide();
    });
  }
  if (p.kind === 'meeting') applyDelta(p);
}

// ---------- data ----------

async function reloadList() {
  try {
    const list = await api('meetingList');
    if (Array.isArray(list)) M.list = list;
    M.listLoaded = true;
  } catch (_) {
    // The list on screen stays.
  }
  renderIndicator();
}

function upsertSummary(s) {
  if (!s || !s.id) return false;
  const i = M.list.findIndex((m) => m.id === s.id);
  const before = i >= 0 ? M.list[i] : null;
  if (i >= 0) M.list[i] = s;
  else {
    M.list.push(s);
    M.list.sort((a, b) => (b.startedAt || 0) - (a.startedAt || 0) || (a.id < b.id ? 1 : -1));
  }
  // The sidebar shows the title, the state and the start.
  return !before || before.title !== s.title || before.state !== s.state || before.startedAt !== s.startedAt || before.removed !== s.removed || before.eventId !== s.eventId;
}

// The meeting to show when the view opens: the one recording, else the one from before, else the newest.
function pickAndOpen() {
  if (!M.listLoaded) return renderView();
  // The recording comes first, also when its first events are still on their way: they fetch it once they arrive.
  const rec = M.voice.recording;
  const known = (id) => id && summaryOf(id);
  const id = rec || (known(M.id) ? M.id : null) || (M.list.find((m) => !m.removed) || M.list[0] || {}).id || null;
  if (id && id !== M.id) return open(id);
  if (id && !M.data) {
    renderView();
    return fetchMeeting(id);
  }
  if (!id) {
    M.id = null;
    M.data = null;
  }
  renderView();
}

let pickTimer = null;
function pickSoon() {
  clearTimeout(pickTimer);
  pickTimer = setTimeout(() => M.open && !M.id && pickAndOpen(), 120);
}

async function fetchMeeting(id) {
  const mine = ++M.loading;
  let data = null;
  try {
    data = await api('meetingGet', id);
  } catch (_) {
    data = null;
  }
  if (mine !== M.loading || id !== M.id) return;
  // Gone from Voice and from Rukoo, notes and all: the view moves on to the next meeting.
  if (!data && M.open && !summaryOf(id) && id !== M.voice.recording) {
    M.id = null;
    M.data = null;
    saveId(null);
    return pickAndOpen();
  }
  // A main from before frames sends none.
  if (data && !Array.isArray(data.frames)) data.frames = [];
  M.data = data;
  M.fetched = id;
  if (data && data.meeting) upsertSummary(data.meeting);
  renderView({ fetched: true });
  ctx.agentSync?.();
}

function setVoice(state) {
  const before = M.voice;
  M.voice = { ...M.voice, ...state };
  const started = M.voice.recording && M.voice.recording !== before.recording;
  if (started && !summaryOf(M.voice.recording)) reloadList().then(() => redrawSide());
  // A recording that starts while the view is open shows at once, unless the user is writing in this one.
  if (started && M.open && M.voice.recording !== M.id && !busyHere()) open(M.voice.recording);
  afterVoice();
  if (before.status !== M.voice.status || before.recording !== M.voice.recording) redrawSide();
  if (M.open) {
    if (M.data) {
      renderHead();
      renderNotice();
      renderLive();
      // Frame images come from Voice: rows that waited for it load now, and the others say why they can't.
      if (before.status !== M.voice.status) redrawFrames();
    } else renderView();
  }
}

function busyHere() {
  const el = document.activeElement;
  return Boolean(M.renaming || M.editing) || Boolean(el && el.closest && el.closest('.mt-notes-input'));
}

// A change to one meeting, batched by main. from is the meeting's rev before it: a view that holds another rev
// missed one, and fetches the meeting again.
function applyDelta(p) {
  const listChanged = upsertSummary(p.meeting);
  if (listChanged) redrawSide();
  renderIndicator();
  if (M.open && p.id === M.id && !M.data) return fetchMeeting(M.id);
  if (p.id !== M.id || !M.data) {
    // The view had nothing to show, and now there is a meeting: once the rest of this batch is in, the newest.
    if (M.open && !M.id) pickSoon();
    return;
  }
  const d = M.data;
  const eventChanged = (p.meeting && p.meeting.eventId) !== d.meeting.eventId;
  if (p.refetch || d.rev !== p.from || eventChanged) return fetchMeeting(M.id);
  d.meeting = p.meeting;
  d.rev = p.rev;
  const changed = [];
  if (Array.isArray(p.passages)) {
    for (const x of p.passages) {
      const i = d.passages.findIndex((y) => y.id === x.id);
      if (i >= 0) d.passages[i] = x;
      else d.passages.push(x);
      changed.push(`p:${x.id}`);
    }
    d.passages.sort((a, b) => a.start - b.start || (a.track < b.track ? -1 : 1));
  }
  const removed = Array.isArray(p.removedPassages) ? p.removedPassages : [];
  if (removed.length) d.passages = d.passages.filter((x) => !removed.includes(x.id));
  if (Array.isArray(p.marks)) {
    for (const k of p.marks) {
      const i = d.marks.findIndex((y) => y.id === k.id);
      if (i >= 0) d.marks[i] = k;
      else d.marks.push(k);
      changed.push(`k:${k.id}`);
    }
  }
  if (Array.isArray(p.frames)) {
    d.frames = d.frames || [];
    for (const f of p.frames) {
      const i = d.frames.findIndex((y) => y.id === f.id);
      if (i >= 0) d.frames[i] = f;
      else d.frames.push(f);
      changed.push(`f:${f.id}`);
    }
    d.frames.sort((a, b) => a.at - b.at);
  }
  if (Array.isArray(p.speakers)) d.speakers = p.speakers;
  if (p.notes) notesArrived(p.notes);
  if (!M.open) return;
  renderHead();
  renderNotice();
  if (Array.isArray(p.speakers)) {
    renderPeople();
    renderTranscript(true);
  } else updateRows(changed, removed.map((id) => `p:${id}`));
  renderLive();
  if (listChanged) ctx.agentSync?.();
}

// ---------- the view ----------

function bind() {
  if (bound) return;
  const el = $('.meeting');
  if (!el) return;
  bound = true;
  el.addEventListener('click', onClick);
  el.addEventListener('dblclick', (e) => {
    const text = e.target.closest('.mt-passage .mt-text');
    if (text) editPassage(text.closest('.mt-row'));
  });
  el.addEventListener('contextmenu', onContextMenu);
  el.addEventListener('keydown', onViewKey);
  el.addEventListener('input', (e) => {
    if (e.target.matches('.mt-edit')) grow(e.target);
  });
  el.addEventListener('focusout', (e) => {
    if (e.target.matches && e.target.matches('.mt-notes-input')) saveNotes();
  });
}

// fetched: the meeting on screen came again from main (a delta it missed): its transcript and notes follow too.
function renderView({ fetched = false } = {}) {
  const el = $('.meeting');
  if (!el || !M.open) return;
  bind();
  const d = M.data;
  if (!d || !d.meeting) {
    delete el.dataset.id;
    // A recording whose first events haven't arrived yet loads until they do.
    const waiting = M.id && (M.fetched !== M.id || M.id === M.voice.recording);
    el.innerHTML = !M.listLoaded || waiting ? loadingHtml() : emptyHtml();
    return;
  }
  if (el.dataset.id !== d.meeting.id) {
    el.dataset.id = d.meeting.id;
    notesEditor?.dispose();
    notesMenu?.();
    el.innerHTML = shellHtml();
    const area = el.querySelector('.mt-notes-input');
    notesEditor = mountRichEditor(el.querySelector('.mt-notes-card'), area, { onChange: notesTyped, cssFormatting: false });
    notesMenu = editMenu(area);
    fillNotes(d);
    renderTranscript(true, { bottom: true });
  } else if (fetched) {
    renderTranscript(true);
    if (d.notes) notesArrived(d.notes);
  }
  el.dataset.tab = M.tab;
  renderHead();
  renderNotice();
  renderPeople();
  renderEvent();
  renderLive();
}

function loadingHtml() {
  return `<div class="mt-empty" aria-busy="true"><span class="spinner"></span></div>`;
}

// Nothing to show: the way to record a meeting, or why Rukoo can't.
function emptyHtml() {
  const v = M.voice;
  let title = t('meeting.empty.startVoice');
  let body = t('meeting.empty.body');
  let after = `<div class="mt-empty-hint">${esc(t('meeting.empty.voiceHint'))}</div>`;
  if (v.status === 'connected') {
    title = t('meeting.empty.title');
    after = `<div class="mt-record-split"><button class="btn primary" data-mt="record">${icons.record}<span>${esc(t('meeting.actions.record'))}</span></button>${recordMoreHtml('btn primary', 'data-mt="record-menu"')}</div><div class="mt-empty-hint">${esc(t('meeting.empty.calendarHint'))}</div>`;
  } else if (v.status === 'connecting') {
    title = t('meeting.voice.connecting');
    after = '';
  } else if (v.status !== 'absent') {
    title = t('meeting.empty.cantConnect');
    body = notReadyText(v);
    after = '';
  }
  return `<div class="mt-empty">${icons.mic}<div class="t">${esc(title)}</div><div class="mt-empty-body">${esc(body)}</div>${after}</div>`;
}

// The arrow half of a Record button, which opens the screen choice (screenMenu).
function recordMoreHtml(cls, data, extra = '') {
  const label = esc(t('meeting.screen.menu'));
  return `<button class="${cls} mt-record-more" ${data} aria-haspopup="menu" aria-label="${label}" title="${label}" ${extra}>${icons.chevronDown}</button>`;
}

function shellHtml() {
  const tabs = segmented({
    name: 'mt-tab',
    label: t('meeting.tabs.label'),
    value: M.tab,
    options: [
      { value: 'transcript', label: t('meeting.tabs.transcript') },
      { value: 'notes', label: t('meeting.tabs.notes') },
      { value: 'people', label: t('meeting.tabs.details') }
    ]
  });
  return `<header class="mt-head"></header>
    <div class="mt-notice" hidden></div>
    <div class="mt-tabs">${tabs}</div>
    <div class="mt-body">
      <section class="mt-transcript-pane" aria-label="${esc(t('meeting.tabs.transcript'))}">
        <div class="mt-transcript-head"><span>${esc(t('meeting.tabs.transcript'))}</span><button class="tbtn" data-mt="show-frames" aria-pressed="${M.showFrames}">${icons.monitor}<span>${esc(t('meeting.frame.visibility'))}</span></button></div>
        <div class="mt-scroll" tabindex="-1">
          <ol class="mt-transcript" tabindex="0" aria-label="${esc(t('meeting.transcript.label'))}"></ol>
          <div class="mt-live" aria-live="polite"></div>
        </div>
        <button class="mt-jump" data-mt="jump" hidden>${icons.down}<span>${esc(t('meeting.transcript.latest'))}</span></button>
      </section>
      <aside class="mt-side">
        <section class="mt-card mt-event-card" data-pane="people">
          <h2 class="mt-card-title">${esc(t('meeting.event.title'))}</h2>
          <div class="mt-event-box"></div>
        </section>
        <section class="mt-card mt-people-card" data-pane="people">
          <h2 class="mt-card-title">${esc(t('meeting.speakers.title'))}</h2>
          <div class="mt-people"></div>
        </section>
        <section class="mt-card mt-notes-card" data-pane="notes">
          <div class="mt-card-head"><h2 class="mt-card-title" id="mt-notes-title">${esc(t('meeting.notes.title'))}</h2><span class="mt-notes-status" aria-live="polite"></span></div>
          <div class="mt-notes-toolbar" role="toolbar" aria-label="${esc(t('composer.format.label'))}">${formatButtons()}<span class="sep"></span><button class="icon-btn sm" data-cmd="formatBlock" data-value="h2" title="${esc(t('meeting.notes.heading'))}" aria-label="${esc(t('meeting.notes.heading'))}">H2</button></div>
          <div class="mt-notes-input editor sel" contenteditable="true" role="textbox" aria-multiline="true" spellcheck="true" aria-labelledby="mt-notes-title" data-placeholder="${esc(t('meeting.notes.placeholder'))}"></div>
        </section>
      </aside>
    </div>`;
}

// ---------- header ----------

function stateHtml(s) {
  if (s.removed) return `<span class="mt-state">${esc(t('meeting.state.removed'))}</span>`;
  const cls = { recording: 'is-rec', paused: 'is-paused', processing: 'is-busy', failed: 'is-failed' }[s.state];
  if (!cls) return '';
  const title = s.state === 'failed' && s.reason ? ` title="${esc(s.reason)}"` : '';
  return `<span class="mt-state ${cls}"${title}><i aria-hidden="true"></i>${esc(t(`meeting.state.${s.state}`))}</span>`;
}

function renderHead() {
  const head = $('.meeting .mt-head');
  const d = M.data;
  if (!head || !d) return;
  const s = d.meeting;
  if (M.renaming === s.id && head.querySelector('.mt-title-input')) return;
  const connected = M.voice.status === 'connected';
  const disabled = connected ? '' : `disabled title="${esc(notReadyText() || '')}"`;
  const ms = elapsed(s);
  const when = Number.isFinite(s.startedAt) ? readerDate(s.startedAt) : '';
  const screen = screenOf(s);
  // Rukoo shows in the frames when it isn't hidden from capture, where it could be.
  const visible = live(s) && screen && M.voice.canHide && !M.voice.hidden;
  const meta = [
    stateHtml(s),
    ms !== null ? `<span class="mt-elapsed" title="${esc(t('meeting.head.length'))}">${esc(live(s) ? clock(ms) : length(ms))}</span>` : '',
    when ? `<span class="mt-when">${esc(when)}</span>` : '',
    screen ? `<span class="mt-screen">${icons.monitor}<span>${esc(t('meeting.head.screen', { mode: screenMode(screen) }))}</span></span>` : '',
    live(s) && M.voice.hidden ? `<span class="mt-hidden" title="${esc(t('meeting.head.hiddenHelp'))}">${icons.eyeOff}<span>${esc(t('meeting.head.hidden'))}</span></span>` : '',
    visible ? `<span class="mt-visible" title="${esc(t('meeting.head.visibleHelp'))}">${icons.eye}<span>${esc(t('meeting.head.visible'))}</span></span>` : ''
  ]
    .filter(Boolean)
    .join('');
  const confirming = M.confirmStop === s.id;
  // A Voice that can't switch monitors (Voice 0.1) answered unknown_command once; its meetings get no button.
  const canSwitch = M.voice.protocol >= 2 && M.noScreenSwitch !== voiceKey();
  const screenLabel = esc(t('meeting.actions.screen'));
  const actions = live(s)
    ? `${canSwitch ? `<button class="tbtn" data-mt="screen" aria-haspopup="menu" title="${screenLabel}" ${disabled}>${icons.monitor}<span>${screenLabel}</span></button>` : ''}
       <button class="tbtn" data-mt="mark" ${disabled}>${icons.flag}<span>${esc(t('meeting.actions.mark'))}</span></button>
       <button class="tbtn" data-mt="${s.state === 'paused' ? 'resume' : 'pause'}" ${disabled}>${s.state === 'paused' ? icons.play : icons.pause}<span>${esc(t(s.state === 'paused' ? 'meeting.actions.resume' : 'meeting.actions.pause'))}</span></button>
       <button class="tbtn primary ${confirming ? 'is-confirm' : ''}" data-mt="stop" ${disabled}>${icons.stop}<span>${esc(t(confirming ? 'meeting.actions.stopConfirm' : 'meeting.actions.stop'))}</span></button>`
    : '';
  const focused = head.contains(document.activeElement) ? document.activeElement.dataset.mt : null;
  const title = titleOf(s, d.event);
  const titleHtml = M.renaming === s.id
    ? `<input class="mt-title-input" aria-label="${esc(t('meeting.actions.rename'))}" value="${esc(title)}" />`
    : `<h1 class="mt-title sel" title="${esc(title)}">${esc(title)}</h1>${!s.removed ? `<button class="icon-btn xs" data-mt="rename" title="${esc(t('meeting.actions.rename'))}" aria-label="${esc(t('meeting.actions.rename'))}">${icons.edit}</button>` : ''}`;
  head.innerHTML = `<div class="mt-titles"><div class="mt-title-row">${titleHtml}</div><div class="mt-meta">${meta}</div></div><div class="mt-actions">${actions}<button class="icon-btn sm" data-mt="more" aria-haspopup="menu" aria-label="${esc(t('meeting.actions.more'))}" title="${esc(t('meeting.actions.more'))}">${icons.more}</button></div>`;
  // A redraw keeps the keyboard where it was; Pause becomes Resume in the same place.
  if (focused) (head.querySelector(`[data-mt="${focused}"]`) || head.querySelector(`[data-mt="${focused === 'pause' ? 'resume' : focused === 'resume' ? 'pause' : ''}"]`))?.focus();
}

function canDelete(s) {
  return s && !['recording', 'paused', 'processing'].includes(s.state) && s.id !== M.voice.recording;
}

function meetingMenu(anchor, id, opts = {}) {
  const s = summaryOf(id);
  if (!s) return;
  showMenu(anchor, [
    !s.removed ? { label: t('meeting.actions.rename'), icon: 'edit', action: async () => { await open(id); beginRename(id); } } : null,
    canDelete(s) ? { label: t('meeting.actions.delete'), icon: 'trash', danger: true, disabled: M.deleting.has(id), action: () => deleteMeeting(id) } : null
  ], opts);
}

function beginRename(id) {
  if (!M.data || M.data.meeting.id !== id || M.data.meeting.removed) return;
  M.renaming = id;
  renderHead();
  const input = $('.meeting .mt-title-input');
  input?.focus();
  input?.select();
}

async function finishRename(save) {
  const id = M.renaming;
  const input = $('.meeting .mt-title-input');
  if (!id || !input || M.renameBusy) return;
  if (!save) {
    M.renaming = null;
    renderHead();
    $('.meeting [data-mt="rename"]')?.focus();
    return;
  }
  const title = input.value.trim();
  if (!title || Array.from(title).length > 200) {
    input.setAttribute('aria-invalid', 'true');
    return toast(t('meeting.rename.invalid'), { ms: 5000 });
  }
  M.renameBusy = true;
  input.disabled = true;
  try {
    const summary = await api('meetingRename', id, title);
    if (M.data?.meeting.id === id) M.data.meeting = summary;
    upsertSummary(summary);
    M.renaming = null;
    renderHead();
    redrawSide();
    ctx.agentSync?.();
    $('.meeting [data-mt="rename"]')?.focus();
  } catch (err) {
    input.disabled = false;
    input.focus();
    toast(voiceError(err), { ms: 5000 });
  } finally {
    M.renameBusy = false;
  }
}

async function deleteMeeting(id) {
  const s = summaryOf(id);
  if (!canDelete(s) || M.deleting.has(id)) return;
  const yes = await confirmDialog(t('meeting.delete.title', { title: titleOf(s) }), t('meeting.delete.body'), t('meeting.actions.delete'), true);
  if (!yes || !canDelete(summaryOf(id))) return;
  M.deleting.add(id);
  try {
    await api('meetingDelete', id);
    N.queued.delete(id);
    N.blocked.delete(id);
    M.renaming = null;
    toast(t('meeting.delete.done'), { ms: 4000 });
  } catch (err) {
    toast(err.voice === 'unknown_command' ? t('meeting.delete.updateVoice') : voiceError(err), { ms: 6000 });
  } finally {
    M.deleting.delete(id);
  }
}

// A line under the header when something about the meeting needs saying.
function renderNotice() {
  const el = $('.meeting .mt-notice');
  const d = M.data;
  if (!el || !d) return;
  const s = d.meeting;
  let text = '';
  if (s.removed) text = t('meeting.notice.removed');
  else if (M.voice.status !== 'connected' && (live(s) || s.state === 'processing')) text = t('meeting.notice.offline');
  else if (s.state === 'failed') text = t('meeting.notice.failed');
  el.hidden = !text;
  el.innerHTML = text ? `${icons.info}<span>${esc(text)}</span>` : '';
}

// ---------- the monitor Voice records ----------

// Which Voice is connected, as far as the Screen button cares: the same version answers the same.
const voiceKey = () => `${M.voice.status}:${M.voice.voiceVersion || ''}`;

// The header's Screen button: the monitors Voice can record, the one it records checked. The menu opens at once and
// fills in when Voice answers.
async function monitorMenu(btn) {
  const d = M.data;
  if (!d) return;
  const id = d.meeting.id;
  const heading = { heading: t('meeting.screen.liveChoose') };
  showMenu(btn, [heading, { label: t('meeting.monitor.loading'), disabled: true, action: () => {} }], { align: 'right' });
  const opened = menuCount();
  // A new line or a Voice status redraws the header while Voice answers; the menu then belongs to the new button.
  const button = () => (btn.isConnected ? btn : $('.meeting .mt-head [data-mt="screen"]'));
  // Our menu is the one open; it fills in only while its meeting is shown with a Screen button.
  const ours = () => menuOpen() && menuCount() === opened;
  const stillOpen = () => ours() && button() && M.data && M.data.meeting.id === id;
  let info = null;
  try {
    info = await api('meetingScreen', id);
  } catch (err) {
    if (ours()) closeMenu({ refocus: Boolean(stillOpen()) });
    if (err && err.voice === 'unknown_command') {
      M.noScreenSwitch = voiceKey();
      renderHead();
    }
    return toast(voiceError(err), 5000);
  }
  if (!stillOpen()) {
    // The recording ended or another meeting opened while Voice answered: the menu would stay on Loading.
    if (ours()) closeMenu();
    return;
  }
  const monitors = info && Array.isArray(info.monitors) ? info.monitors.filter((m) => m && typeof m.id === 'string') : [];
  const stopped = !info || info.state === 'stopped';
  const remembered = M.monitors.get(id);
  if (remembered && !monitors.some((m) => m.id === remembered)) M.monitors.delete(id);
  // Voice may have switched from its pill or fallen back after a display disconnected.
  const selected = info.monitor || M.monitors.get(id);
  showMenu(
    button(),
    [
      heading,
      ...SCREENS.map((mode) => ({
        label: t(`meeting.screen.control.${mode}`),
        checked: info.screen === mode,
        disabled: stopped,
        action: () => setScreenMode(id, mode, mode === 'off' ? null : selected)
      })),
      { separator: true },
      { heading: t('meeting.monitor.heading') },
      stopped ? { label: t('meeting.monitor.stopped'), disabled: true, action: () => {} } : null,
      ...monitors.map((m) => ({
        label: m.name || m.id,
        sub: Number.isInteger(m.width) && Number.isInteger(m.height) ? t('meeting.monitor.size', { width: m.width, height: m.height }) : '',
        hint: m.primary ? t('meeting.monitor.primary') : '',
        checked: m.id === selected,
        disabled: stopped,
        action: () => {
          M.monitors.set(id, m.id);
          if (m.id !== info.monitor) setScreenMode(id, info.screen, m.id);
        }
      }))
    ],
    { align: 'right' }
  );
}

async function setScreenMode(id, mode, monitor) {
  try {
    const result = await api('meetingScreenSet', id, mode, monitor);
    if (M.data?.meeting.id === id) {
      M.data.meeting.screen = result.screen;
      upsertSummary(M.data.meeting);
      renderHead();
    }
  } catch (err) {
    if (err.voice === 'unknown_command') {
      M.noScreenSwitch = voiceKey();
      renderHead();
    }
    toast(voiceError(err), { ms: 6000 });
  }
}

// The line after the transcript: listening while it records, finishing once it stopped.
function renderLive() {
  const el = $('.meeting .mt-live');
  const d = M.data;
  if (!el || !d) return;
  const s = d.meeting;
  const none = !d.passages.length;
  let html = '';
  if (s.state === 'recording') html = `<span class="mt-listening">${esc(t(none ? 'meeting.live.waiting' : 'meeting.live.listening'))}</span>`;
  else if (s.state === 'paused') html = `<span>${esc(t('meeting.live.paused'))}</span>`;
  else if (s.state === 'processing') html = `<span class="spinner"></span><span>${esc(t('meeting.live.processing'))}</span>`;
  else if (none && !s.removed) html = `<span>${esc(t('meeting.live.nothing'))}</span>`;
  el.innerHTML = html;
  el.className = `mt-live ${s.state === 'recording' ? 'is-rec' : ''}`;
}

// ---------- transcript ----------

function speakerOf(id) {
  return (M.data && M.data.speakers.find((s) => s.id === id)) || null;
}
// A passage's voice: its speaker, or the user for the microphone when Voice hasn't said whose it is.
function voiceOf(p) {
  const s = speakerOf(p.speaker);
  if (s) return s;
  return p.track === 'mic' ? { id: 'mic', track: 'mic', you: true, label: null, name: null } : { id: null, track: p.track, label: null, name: null };
}
// The N of a call voice Voice labelled "Speaker N" under the id call-N, else null.
function speakerNumber(s) {
  const m = s.track === 'call' && typeof s.id === 'string' ? /^call-([1-9]\d*)$/.exec(s.id) : null;
  return m && s.label === `Speaker ${m[1]}` ? m[1] : null;
}
// What Voice calls a voice, in the interface's language: Voice labels call-N "Speaker N" in English, so that label
// becomes "Spreker N" in Dutch. Any other label shows as Voice sent it.
function speakerLabel(s) {
  const n = speakerNumber(s);
  return n ? t('meeting.speakers.numbered', { n }) : s.label || t('meeting.speakers.unknown');
}
function speakerName(s) {
  if (s.name) return s.name;
  if (s.you || s.track === 'mic') return t('meeting.speakers.you');
  return speakerLabel(s);
}
// An unnamed voice on the call shows its number, so Speaker 1 and Speaker 10 don't both read "S1". Its color follows
// the number around the color wheel by the golden angle (Speaker 1 blue, 2 rose, 3 green, ...): hashing the label would
// give Speaker 1 and Speaker 2 nearly the same color, and change it with the language.
function avatarHtml(s) {
  if (s.you || s.track === 'mic') return `<span class="mt-av is-you" aria-hidden="true">${icons.mic}</span>`;
  const name = speakerName(s);
  const n = s.name ? null : speakerNumber(s);
  const h = n ? (Number(n) * 137 + 83) % 360 : hue(s.address || name);
  return `<span class="mt-av" style="--h:${h}" aria-hidden="true">${esc(n || initials({ name }))}</span>`;
}

const TRACK_KEYS = { mic: 'meeting.track.mic', call: 'meeting.track.call', screen: 'meeting.track.screen' };
const trackName = (track) => t(TRACK_KEYS[track] || 'meeting.track.all');

// What a mark says. Device names come from Voice's device list; the label of a user mark is what Voice was given.
function markText(k) {
  const span = Number.isFinite(k.until) ? length(k.until - k.at) : null;
  const track = trackName(k.track);
  switch (k.kind) {
    case 'screen_mode':
      return t(`meeting.mark.screen.${SCREENS.includes(k.label) ? k.label : 'off'}`);
    case 'user':
      return k.label ? t('meeting.mark.userLabel', { label: k.label }) : t('meeting.mark.user');
    case 'dictation':
      return span ? t('meeting.mark.dictation', { length: span }) : t('meeting.mark.dictationOpen');
    case 'gap': {
      const key = { crash: 'meeting.mark.gapCrash', device: 'meeting.mark.gapDevice', suspend: 'meeting.mark.gapSuspend' }[k.detail] || 'meeting.mark.gap';
      return t(key, { track, length: span || length(0) });
    }
    case 'device': {
      const device = k.label || t('meeting.mark.unnamedDevice');
      const key = { reconnected: 'meeting.mark.deviceBack', lost: 'meeting.mark.deviceLost' }[k.detail] || 'meeting.mark.deviceMoved';
      return t(key, { track, device });
    }
    // The whole screen recording stopped; the audio goes on (voice/PROTOCOL.md, marks).
    case 'dropped': {
      const key = { disk: 'meeting.mark.droppedDisk', device: 'meeting.mark.droppedDevice', crash: 'meeting.mark.droppedCrash' }[k.detail] || 'meeting.mark.droppedEncoder';
      return t(key);
    }
    // Only the video stopped. After a lost graphics device the frames may stop right after; their own mark says so.
    case 'video_dropped': {
      const key = { disk: 'meeting.mark.droppedVideoDisk', device: 'meeting.mark.droppedVideoDevice' }[k.detail] || 'meeting.mark.droppedVideoEncoder';
      return t(key);
    }
    // Monitor names come from Windows, through Voice.
    case 'monitor': {
      const name = k.label || t('meeting.mark.unnamedDevice');
      return k.detail === 'lost' ? t('meeting.mark.monitorLost', { name }) : t('meeting.mark.monitor', { name });
    }
    // Voice heard only exact silence on the call while another app played sound: on a Mac, System Audio Recording may be off.
    case 'silent':
      return span ? t('meeting.mark.silentUntil', { track, length: span }) : t('meeting.mark.silent', { track });
    // A stretch without frames; a gap stays about missing audio.
    case 'screen_gap': {
      const key = { locked: 'meeting.mark.gapScreenLocked', crash: 'meeting.mark.gapScreenCrash', device: 'meeting.mark.gapScreenDevice', monitor: 'meeting.mark.gapScreenMonitor' }[k.detail] || 'meeting.mark.gapScreen';
      return t(key, { length: span || length(0) });
    }
    default:
      return k.label ? t('meeting.mark.userLabel', { label: k.label }) : t('meeting.mark.user');
  }
}
const MARK_ICONS = { user: 'flag', gap: 'warning', device: 'mic', dictation: 'edit', dropped: 'warning', video_dropped: 'warning', screen_gap: 'warning', screen_mode: 'monitor', monitor: 'monitor', silent: 'warning' };

function markHtml(k) {
  const kind = Object.hasOwn(MARK_ICONS, k.kind) ? k.kind : 'user';
  return `<li class="mt-row mt-mark is-${kind}" data-key="k:${esc(k.id)}" data-at="${k.at}" tabindex="-1">
    <time class="mt-at">${esc(clock(k.at))}</time>
    <div class="mt-line">${icons[MARK_ICONS[kind]]}<span>${esc(markText(k))}</span></div>
  </li>`;
}

function passageHtml(p) {
  const s = voiceOf(p);
  const wall = M.data && Number.isFinite(M.data.meeting.startedAt) ? hhmm(M.data.meeting.startedAt + p.start) : '';
  const cls = ['mt-row', 'mt-passage', p.dictation ? 'is-dict' : '', p.corrected ? 'is-corrected' : ''].filter(Boolean).join(' ');
  return `<li class="${cls}" data-key="p:${esc(p.id)}" data-id="${esc(p.id)}" data-voice="${esc(s.id || p.track)}" data-at="${p.start}" data-end="${p.end}" tabindex="-1">
    <time class="mt-at"${wall ? ` title="${esc(wall)}"` : ''}>${esc(clock(p.start))}</time>
    <div class="mt-main">
      <div class="mt-who">${avatarHtml(s)}<span class="mt-name">${esc(speakerName(s))}</span>${p.dictation ? `<span class="mt-tag" title="${esc(t('meeting.transcript.dictatingHelp'))}">${icons.edit}<span>${esc(t('meeting.transcript.dictating'))}</span></span>` : ''}</div>
      <div class="mt-text sel">${esc(p.text)}</div>
      ${p.corrected ? `<div class="mt-corrected">${esc(t('meeting.transcript.corrected'))}</div>` : ''}
    </div>
  </li>`;
}

// A moment the screen changed: a thumbnail, filled in from Voice once the row comes into view (fillFrames), and the
// first line of the text Voice read on the screen. A click on the thumbnail or the toggle, or Enter on the row,
// opens it: the image at the row's width and all of its text. The text is whatever was on the screen: esc() only.
function frameHtml(f, group = [f]) {
  const entry = F.cache.get(f.id) || null;
  const text = frameText(f, entry);
  const lines = text === null ? [] : text.split('\n').map((line) => line.trim()).filter(Boolean);
  const open = M.frameOpen.has(f.id);
  const note = frameNote(entry);
  const ratio = Number.isInteger(f.width) && Number.isInteger(f.height) && f.width > 0 && f.height > 0 ? `${f.width} / ${f.height}` : '16 / 9';
  const label = esc(t('meeting.frame.alt', { time: clock(f.at) }));
  const first = lines.length
    ? `<div class="mt-frame-line sel">${esc(lines[0])}</div>`
    : text === ''
      ? `<div class="mt-frame-line is-none">${esc(t('meeting.frame.noText'))}</div>`
      : '';
  const rest =
    lines.length > 1
      ? `<button class="mt-frame-toggle" data-mt="frame" aria-expanded="${open}">${icons.chevronDown}<span>${esc(t('meeting.frame.showText'))}</span></button>
        <div class="mt-frame-rest"><div class="mt-frame-full sel">${esc(lines.slice(1).join('\n'))}</div></div>`
      : '';
  return `<li class="mt-row mt-frame${open ? ' is-open' : ''}" data-key="f:${esc(f.id)}" data-id="${esc(f.id)}" data-at="${f.at}" tabindex="-1">
    <time class="mt-at">${esc(clock(f.at))}</time>
    <div class="mt-main">
      <button class="mt-shot" data-mt="frame" aria-expanded="${open}" aria-label="${label}" style="--ar: ${ratio}">${icons.monitor}<img alt="${label}" loading="lazy" decoding="async" hidden></button>
      <div class="mt-frame-body">${first}${note ? `<div class="mt-frame-note">${esc(note)}</div>` : ''}${rest}${group.length > 1 ? `<button class="mt-frame-count" data-mt="frame-group" aria-expanded="${M.groupOpen.has(f.id)}" aria-label="${esc(t('meeting.frame.group', { count: group.length - 1 }))}">+${group.length - 1}</button>` : ''}</div>
      ${group.length > 1 ? `<ol class="mt-frame-strip" ${M.groupOpen.has(f.id) ? '' : 'hidden'}>${group.slice(1).map((x) => frameHtml(x)).join('')}</ol>` : ''}
    </div>
  </li>`;
}

// Marks before a passage that starts at the same moment. A dictation stretch whose words are in the transcript
// already shows as those lines, tagged; its mark only says when nothing was transcribed in it.
function items(d) {
  const out = [];
  const dictated = (k) => d.passages.some((p) => p.dictation && p.start < (Number.isFinite(k.until) ? k.until : Infinity) && p.end > k.at);
  for (const k of d.marks) if (!(k.kind === 'dictation' && dictated(k))) out.push({ key: `k:${k.id}`, at: k.at, order: 0, html: () => markHtml(k) });
  // A frame sits at the moment the screen changed, after a mark and before a passage that start then.
  if (M.showFrames) for (const group of foldFrames(d.frames || [])) {
    const f = group[0];
    out.push({ key: `f:${f.id}`, at: f.at, order: 0.5, html: () => frameHtml(f, group) });
  }
  for (const p of d.passages) out.push({ key: `p:${p.id}`, at: p.start, order: 1, html: () => passageHtml(p) });
  return out.sort((a, b) => a.at - b.at || a.order - b.order || (a.key < b.key ? -1 : 1));
}
const rowKey = (el) => [Number(el.dataset.at), el.classList.contains('mt-mark') ? 0 : el.classList.contains('mt-frame') ? 0.5 : 1, el.dataset.key];
const before = (a, b) => a[0] - b[0] || a[1] - b[1] || (a[2] < b[2] ? -1 : a[2] > b[2] ? 1 : 0);

const scroller = () => $('.meeting .mt-scroll');
const atBottom = (el) => !el || el.scrollHeight - el.scrollTop - el.clientHeight < 48;

// The whole transcript, drawn again: when the meeting opens and when names change.
function renderTranscript(full, { bottom = false } = {}) {
  const list = $('.meeting .mt-transcript');
  const d = M.data;
  if (!list || !d || !full) return;
  forgetFrames(d.meeting.id);
  const sc = scroller();
  const stick = bottom || atBottom(sc);
  const top = sc ? sc.scrollTop : 0;
  const focusKey = list.contains(document.activeElement) ? document.activeElement.dataset.key : null;
  const draft = list.querySelector('.mt-edit');
  const draftText = draft ? draft.value : null;
  list.innerHTML = items(d)
    .map((x) => x.html())
    .join('');
  continuations();
  fillFrames();
  if (M.editing) {
    const row = list.querySelector(`[data-key="p:${CSS.escape(M.editing)}"]`);
    if (row) {
      const editing = M.editing;
      M.editing = null;
      editPassage(row);
      const area = row.querySelector('.mt-edit');
      if (area && draftText !== null && M.editing === editing) {
        area.value = draftText;
        grow(area);
      }
    } else M.editing = null;
  }
  if (focusKey) list.querySelector(`[data-key="${CSS.escape(focusKey)}"]`)?.focus({ preventScroll: true });
  if (sc) {
    if (stick) sc.scrollTop = sc.scrollHeight;
    else sc.scrollTop = top;
  }
}

// Rows that changed or arrived go in their place by time; the rest stay, so focus and selection survive.
function updateRows(keys, removedKeys) {
  const list = $('.meeting .mt-transcript');
  const d = M.data;
  if (!list || !d) return;
  forgetFrames(d.meeting.id);
  const sc = scroller();
  const stick = atBottom(sc);
  for (const key of removedKeys) list.querySelector(`[data-key="${CSS.escape(key)}"]`)?.remove();
  const byKey = new Map(items(d).map((x) => [x.key, x]));
  // A new frame can change an earlier group's count or split it after OCR arrives.
  if (keys.some((key) => key.startsWith('f:'))) keys = [...keys, ...[...byKey.keys()].filter((key) => key.startsWith('f:'))];
  // A row whose item is gone, such as a dictation mark once its words arrived.
  for (const row of [...list.children]) if (!byKey.has(row.dataset.key)) row.remove();
  let added = false;
  for (const key of new Set(keys)) {
    const item = byKey.get(key);
    if (!item) continue;
    if (key === `p:${M.editing}`) continue;
    const tmp = document.createElement('template');
    tmp.innerHTML = item.html().trim();
    const row = tmp.content.firstElementChild;
    const old = list.querySelector(`[data-key="${CSS.escape(key)}"]`);
    const hadFocus = old && old === document.activeElement;
    const active = old?.contains(document.activeElement) ? document.activeElement : null;
    const activeFrame = active?.closest('.mt-frame')?.dataset.id;
    const activeAction = active?.dataset.mt;
    if (old) old.remove();
    else added = true;
    const k = rowKey(row);
    // Passages mostly arrive in order: look from the end.
    let next = null;
    for (let el = list.lastElementChild; el; el = el.previousElementSibling) {
      if (before(rowKey(el), k) <= 0) break;
      next = el;
    }
    list.insertBefore(row, next);
    if (hadFocus) row.focus({ preventScroll: true });
    else if (activeFrame && activeAction) {
      const frame = row.dataset.id === activeFrame ? row : row.querySelector(`[data-id="${CSS.escape(activeFrame)}"]`);
      frame?.querySelector(`[data-mt="${CSS.escape(activeAction)}"]`)?.focus({ preventScroll: true });
    }
  }
  continuations();
  fillFrames();
  if (sc && stick) sc.scrollTop = sc.scrollHeight;
  else if (added) $('.meeting .mt-jump')?.removeAttribute('hidden');
}

// A passage by the same voice soon after the one before it doesn't repeat the name.
function continuations() {
  let prev = null;
  for (const row of $('.meeting .mt-transcript')?.children || []) {
    const passage = row.classList.contains('mt-passage');
    const cont = passage && prev && prev.classList.contains('mt-passage') && prev.dataset.voice === row.dataset.voice && prev.classList.contains('is-dict') === row.classList.contains('is-dict') && Number(row.dataset.at) - Number(prev.dataset.end) < SAME_STRETCH_MS;
    row.classList.toggle('cont', Boolean(cont));
    prev = row;
  }
}

// ---------- screen frames ----------

// The images of the meeting on screen, by frame id, at most FRAME_CACHE of them; an entry is {url, text} once Voice
// answered, {gone} when Voice no longer has the image and {offline} when it couldn't be asked. loading: frames asked
// for now. io: watches the rows whose image isn't there yet.
const F = { meeting: null, cache: new Map(), loading: new Set(), io: null, root: null };

function forgetFrames(id) {
  if (F.meeting === id) return;
  F.meeting = id;
  F.cache = new Map();
  F.loading = new Set();
}

function remember(meetingId, frameId, entry) {
  if (F.meeting !== meetingId) return;
  F.cache.delete(frameId);
  F.cache.set(frameId, entry);
  while (F.cache.size > FRAME_CACHE) {
    const old = F.cache.keys().next().value;
    F.cache.delete(old);
    releaseImage(old);
  }
}

// An image that left the cache leaves its row too, so FRAME_CACHE bounds the images the view holds; the row is watched
// again and loads the image when it comes back into view. A row in or near view keeps its image: dropping that one
// would only load it again at once.
function releaseImage(frameId) {
  const row = $('.meeting .mt-transcript')?.querySelector(`[data-key="f:${CSS.escape(frameId)}"]`);
  if (!row || !row.classList.contains('has-image') || nearView(row)) return;
  const img = row.querySelector('.mt-shot img');
  if (img) {
    img.removeAttribute('src');
    img.hidden = true;
  }
  row.classList.remove('has-image');
  if (F.io && M.voice.status === 'connected') F.io.observe(row);
}

// Within FRAME_MARGIN_PX of the transcript's visible part, as the observer that loads images counts it.
function nearView(row) {
  const sc = scroller();
  if (!sc) return false;
  const view = sc.getBoundingClientRect();
  const box = row.getBoundingClientRect();
  return box.bottom > view.top - FRAME_MARGIN_PX && box.top < view.bottom + FRAME_MARGIN_PX;
}

// A frame's text: its own, or, when the snapshot left it out to stay small, what meetingFrame brought. null while
// Rukoo doesn't know it yet.
function frameText(f, entry) {
  if (typeof f.text === 'string' && (f.text || !f.textOmitted)) return f.text;
  return entry && typeof entry.text === 'string' ? entry.text : null;
}

// Why a frame shows no image, or ''.
function frameNote(entry) {
  if (entry && entry.url) return '';
  if (entry && entry.gone) return t('meeting.frame.gone');
  if (M.voice.status !== 'connected' || (entry && entry.offline)) return t('meeting.frame.offline');
  return '';
}

const plainImage = (url) => (typeof url === 'string' && /^data:image\/(?:jpeg|png|webp);base64,[A-Za-z0-9+/]+=*$/.test(url) ? url : null);

// meetingFrame answers with the image as a data: URL, or with {url, text} when it brings text Rukoo left out of its
// copy. null means Voice sent something that isn't a plain image.
function frameResult(res) {
  if (typeof res === 'string') return plainImage(res) ? { url: res, text: null } : { url: null, text: null, gone: true };
  if (res && typeof res === 'object') {
    const text = typeof res.text === 'string' ? res.text : res.frame && typeof res.frame.text === 'string' ? res.frame.text : null;
    const url = plainImage(res.url);
    return url ? { url, text } : { url: null, text, gone: true };
  }
  return { url: null, text: null, gone: true };
}

function showImage(row, url) {
  const img = row.querySelector('.mt-shot img');
  if (!img) return;
  if (img.getAttribute('src') !== url) img.src = url;
  img.hidden = false;
  row.classList.add('has-image');
}

// After the transcript drew: rows whose image Rukoo has get it, the others are watched until they come into view.
function fillFrames() {
  const list = $('.meeting .mt-transcript');
  if (!list || !M.data) return;
  const sc = scroller();
  if (F.io) F.io.disconnect();
  if (F.root !== sc) F.io = null;
  if (!F.io && sc && typeof IntersectionObserver === 'function') {
    F.root = sc;
    F.io = new IntersectionObserver(framesSeen, { root: sc, rootMargin: FRAME_MARGIN });
  }
  for (const row of list.querySelectorAll('.mt-frame:not(.has-image)')) {
    const entry = F.cache.get(row.dataset.id);
    if (entry && entry.url) {
      remember(F.meeting, row.dataset.id, entry);
      showImage(row, entry.url);
    } else if (!entry && M.voice.status === 'connected' && F.io) F.io.observe(row);
  }
}

function framesSeen(seen) {
  for (const e of seen) {
    if (!e.isIntersecting) continue;
    F.io.unobserve(e.target);
    loadFrame(e.target.dataset.id);
  }
}

async function loadFrame(frameId) {
  const d = M.data;
  const f = d && (d.frames || []).find((x) => x.id === frameId);
  if (!f || F.loading.has(frameId) || F.cache.has(frameId) || F.meeting !== d.meeting.id) return;
  const meetingId = d.meeting.id;
  F.loading.add(frameId);
  let entry;
  try {
    entry = frameResult(await api('meetingFrame', meetingId, f.at));
  } catch (err) {
    const code = err && err.voice;
    entry = code === 'not_available' || code === 'not_found' ? { url: null, text: null, gone: true } : { url: null, text: null, offline: true };
  } finally {
    F.loading.delete(frameId);
  }
  if (F.meeting !== meetingId) return;
  remember(meetingId, frameId, entry);
  const row = $('.meeting .mt-transcript')?.querySelector(`[data-key="f:${CSS.escape(frameId)}"]`);
  if (!row || !M.data || M.data.meeting.id !== meetingId) return;
  // The image goes in place; a note or text that just arrived draws the row again.
  if (entry.url && frameText(f, null) !== null) showImage(row, entry.url);
  else updateRows([`f:${frameId}`], []);
}

// Voice came or went: rows that couldn't ask it try again, and the notes follow.
function redrawFrames() {
  if (!M.data || !(M.data.frames || []).length) return;
  for (const [id, entry] of [...F.cache]) if (entry.offline) F.cache.delete(id);
  renderTranscript(true);
}

function toggleFrame(row) {
  if (!row) return;
  const id = row.dataset.id;
  const open = !M.frameOpen.has(id);
  if (open) M.frameOpen.add(id);
  else M.frameOpen.delete(id);
  row.classList.toggle('is-open', open);
  for (const b of row.querySelectorAll('[data-mt="frame"]')) b.setAttribute('aria-expanded', String(open));
  // A frame opened near the bottom of the view grows below it; the image comes into view, as a row does with the
  // arrow keys.
  if (open) row.scrollIntoView({ block: 'nearest' });
}

// ---------- correcting a passage ----------

function passageById(id) {
  return M.data ? M.data.passages.find((p) => p.id === id) || null : null;
}

function grow(area) {
  area.style.height = 'auto';
  area.style.height = `${area.scrollHeight + 2}px`;
}

function editPassage(row) {
  if (!row || !row.classList.contains('mt-passage') || !M.data || M.data.meeting.removed) return;
  const p = passageById(row.dataset.id);
  if (!p) return;
  if (M.editing && M.editing !== p.id) closeEditor(false);
  M.editing = p.id;
  const main = row.querySelector('.mt-main');
  if (main.querySelector('.mt-edit')) return main.querySelector('.mt-edit').focus();
  const text = main.querySelector('.mt-text');
  const box = document.createElement('div');
  box.className = 'mt-edit-box';
  box.innerHTML = `<textarea class="mt-edit sel" rows="1" aria-label="${esc(t('meeting.transcript.correctLabel'))}"></textarea>
    <div class="mt-edit-bar"><span class="mt-edit-hint">${esc(t('meeting.transcript.correctHint'))}</span><button class="btn sm secondary" data-mt="edit-cancel">${esc(t('common.actions.cancel'))}</button><button class="btn sm primary" data-mt="edit-save">${esc(t('common.actions.save'))}</button></div>`;
  const area = box.querySelector('textarea');
  area.value = p.text;
  text.replaceWith(box);
  grow(area);
  area.focus();
  area.setSelectionRange(area.value.length, area.value.length);
}

// save: send the text to main; false just closes the editor. Either way the row is drawn from the data again.
async function closeEditor(save) {
  const id = M.editing;
  if (!id) return;
  const list = $('.meeting .mt-transcript');
  const row = list && list.querySelector(`[data-key="p:${CSS.escape(id)}"]`);
  const area = row && row.querySelector('.mt-edit');
  const value = area ? area.value : null;
  M.editing = null;
  if (save && value !== null && M.data) {
    const p = passageById(id);
    const original = p ? (p.corrected ? p.original : p.text) : null;
    const next = value.trim();
    const meetingId = M.data.meeting.id;
    if (p && next !== p.text) {
      // Back to what Voice heard, the correction goes. The row shows the new text now; main's change follows.
      const value = next === original || !next ? null : next;
      Object.assign(p, value === null ? { text: original, corrected: false, original: undefined } : { text: value, corrected: true, original });
      redrawRow(id);
      try {
        await api('meetingCorrect', meetingId, id, value);
      } catch (err) {
        toast(voiceError(err), 5000);
        if (M.data && M.data.meeting.id === meetingId) fetchMeeting(meetingId);
      }
      return;
    }
  }
  if (row) redrawRow(id);
}

function redrawRow(id) {
  updateRows([`p:${id}`], []);
  $('.meeting .mt-transcript')?.querySelector(`[data-key="p:${CSS.escape(id)}"]`)?.focus({ preventScroll: true });
}

async function restorePassage(id) {
  if (!M.data) return;
  try {
    await api('meetingCorrect', M.data.meeting.id, id, null);
  } catch (err) {
    toast(voiceError(err), 5000);
  }
}

function onContextMenu(e) {
  const row = e.target.closest('.mt-passage');
  if (!row || row.querySelector('.mt-edit') || !M.data || M.data.meeting.removed) return;
  // Text the user selected keeps the system menu, with Copy.
  if (String(window.getSelection()).trim()) return;
  e.preventDefault();
  const p = passageById(row.dataset.id);
  if (!p) return;
  showMenu(
    row,
    [
      { label: t('meeting.transcript.correct'), icon: 'edit', action: () => editPassage(row) },
      p.corrected ? { label: t('meeting.transcript.restore'), icon: 'undo', action: () => restorePassage(p.id) } : null
    ],
    { at: { x: e.clientX, y: e.clientY } }
  );
}

// ---------- people ----------

function renderPeople() {
  const box = $('.meeting .mt-people');
  const d = M.data;
  if (!box || !d) return;
  // Main sends them in order (call-2 before call-10); the microphone goes first whatever happens.
  const speakers = [...d.speakers].sort((a, b) => Number(b.you || b.track === 'mic') - Number(a.you || a.track === 'mic'));
  if (!speakers.length) {
    box.innerHTML = d.meeting.removed ? '' : `<p class="mt-quiet">${esc(t('meeting.speakers.none'))}</p>`;
    return;
  }
  const focused = box.contains(document.activeElement) ? document.activeElement.dataset.speaker : null;
  box.innerHTML = speakers
    .map((s) => {
      const name = speakerName(s);
      if (s.you || s.track === 'mic') {
        return `<div class="mt-person is-you">${avatarHtml(s)}<span class="mt-person-text"><span class="mt-person-name">${esc(name)}</span><span class="mt-person-sub">${esc(t('meeting.speakers.microphone'))}</span></span></div>`;
      }
      // A name Rukoo guessed for a 1:1 asks to be checked once a second voice is on the call.
      const check = Boolean(s.name && s.autoCheck);
      const sub = check
        ? t('meeting.speakers.autoCheck')
        : s.name
          ? s.address || (s.label ? t('meeting.speakers.was', { label: speakerLabel(s) }) : '')
          : (s.suggestions || []).length
            ? t('meeting.speakers.pick')
            : t('meeting.speakers.nameIt');
      const subClass = check ? 'is-check' : s.name ? '' : 'is-ask';
      const disabled = d.meeting.removed ? 'disabled' : '';
      return `<button class="mt-person" data-mt="speaker" data-speaker="${esc(s.id)}" aria-haspopup="menu" ${disabled} title="${esc(t('meeting.speakers.choose', { name }))}">${avatarHtml(s)}<span class="mt-person-text"><span class="mt-person-name">${esc(name)}</span><span class="mt-person-sub ${subClass}">${esc(sub)}</span></span>${icons.chevronDown}</button>`;
    })
    .join('');
  if (focused) box.querySelector(`[data-speaker="${CSS.escape(focused)}"]`)?.focus();
}

function speakerMenu(btn) {
  const d = M.data;
  const s = d && d.speakers.find((x) => x.id === btn.dataset.speaker);
  if (!s) return;
  const event = d.event && !d.event.missing ? d.event : null;
  // A guest another voice already has says so, and main lists it after the free ones. It stays a choice: one person
  // can sound like two voices.
  const taker = (c) => (Array.isArray(c.takenBy) && c.takenBy.length ? d.speakers.find((x) => x.id === c.takenBy[0]) || null : null);
  const options = (s.suggestions || []).map((c) => ({
    label: c.name || c.address,
    sub: taker(c) ? t('meeting.speakers.taken', { name: speakerName(taker(c)) }) : c.name && c.address ? c.address : '',
    checked: Boolean(s.name) && s.name === (c.name || c.address) && (s.address || '') === (c.address || ''),
    action: () => setSpeaker(s.id, { name: c.name || c.address, address: c.address || '' })
  }));
  showMenu(
    btn,
    [
      options.length ? { heading: event ? t('meeting.speakers.fromEvent', { title: event.title || t('calendar.event.noTitle') }) : t('meeting.speakers.guests') } : null,
      ...options,
      options.length ? { separator: true } : null,
      { label: t('meeting.speakers.someoneElse'), icon: 'edit', action: () => nameSpeaker(s) },
      s.name ? { label: t('meeting.speakers.clear', { label: speakerLabel(s) }), icon: 'undo', action: () => setSpeaker(s.id, null) } : null
    ],
    { align: 'left' }
  );
}

async function nameSpeaker(s) {
  const value = await promptDialog(t('meeting.speakers.nameTitle', { label: speakerLabel(s) }), s.name || '', { placeholder: t('meeting.speakers.namePlaceholder') });
  if (typeof value !== 'string' || !value.trim()) return;
  setSpeaker(s.id, { name: value.trim(), address: '' });
}

async function setSpeaker(speakerId, value) {
  if (!M.data) return;
  const id = M.data.meeting.id;
  try {
    const speakers = await api('meetingSetSpeaker', id, speakerId, value);
    if (!M.data || M.data.meeting.id !== id || !Array.isArray(speakers)) return;
    M.data.speakers = speakers;
    renderPeople();
    renderTranscript(true);
  } catch (err) {
    toast(voiceError(err), 5000);
  }
}

// ---------- the calendar event ----------

function renderEvent() {
  const box = $('.meeting .mt-event-box');
  const d = M.data;
  if (!box || !d) return;
  const ev = d.event;
  const change = d.meeting.removed ? '' : `<button class="btn sm secondary" data-mt="event" aria-haspopup="menu">${esc(t(ev ? 'meeting.event.change' : 'meeting.event.link'))}</button>`;
  let main;
  if (!ev) main = `<div class="mt-quiet">${esc(t('meeting.event.none'))}</div>`;
  else if (ev.missing) main = `<div class="mt-quiet">${esc(t('meeting.event.missing'))}</div>`;
  else main = `<button class="mt-event-title" data-mt="open-event" title="${esc(t('meeting.event.open'))}">${esc(ev.title || t('calendar.event.noTitle'))}</button><div class="mt-event-when">${esc(eventWhen(ev))}</div>`;
  const focused = box.contains(document.activeElement) ? document.activeElement.dataset.mt : null;
  box.innerHTML = `<span class="mt-event-ic">${icons.calendar}</span><div class="mt-event-text">${main}</div>${change}`;
  if (focused) box.querySelector(`[data-mt="${focused}"]`)?.focus();
}

async function eventMenu(btn) {
  const d = M.data;
  if (!d) return;
  const id = d.meeting.id;
  let found = { events: [], failed: [] };
  try {
    found = await api('meetingEvents', id);
  } catch (_) {
    // The menu says nothing fits; Unlink still works.
  }
  if (!M.data || M.data.meeting.id !== id || !btn.isConnected) return;
  const currentId = d.meeting.eventId;
  const events = (found.events || []).map((e) => ({
    label: e.title || t('calendar.event.noTitle'),
    sub: eventWhen(e),
    checked: e.id === currentId,
    action: () => setEvent(e.id)
  }));
  showMenu(
    btn,
    [
      { heading: t('meeting.event.around') },
      ...(events.length ? events : [{ label: t('meeting.event.noneFound'), disabled: true, action: () => {} }]),
      (found.failed || []).length ? { label: t('meeting.event.someFailed'), disabled: true, action: () => {} } : null,
      currentId ? { separator: true } : null,
      currentId ? { label: t('meeting.event.unlink'), icon: 'close', action: () => setEvent(null) } : null
    ],
    { align: 'right' }
  );
}

async function setEvent(eventId) {
  if (!M.data) return;
  const id = M.data.meeting.id;
  try {
    await api('meetingSetEvent', id, eventId);
    // The meeting's delta follows with the new event; fetch now so the card and the names change together.
    if (M.data && M.data.meeting.id === id) await fetchMeeting(id);
  } catch (err) {
    toast(voiceError(err), 5000);
  }
}

// ---------- notes ----------

// The notes editor saves a moment after typing stops and when it loses focus, with the revision it started from.
// queued holds texts waiting while another save runs, by meeting: captured when the save was asked for, so opening
// another meeting meanwhile can't lose what was typed in the one before.
// blocked holds oversized drafts for the current session; they leave the saved copy untouched.
let notesEditor = null;
let notesMenu = null;
const N = { id: null, rev: 0, dirty: false, saving: false, queued: new Map(), blocked: new Map(), timer: null };

function fillNotes(d) {
  const area = $('.meeting .mt-notes-input');
  if (!area) return;
  N.id = d.meeting.id;
  N.rev = d.notes ? d.notes.rev : 0;
  N.dirty = false;
  const blocked = N.blocked.get(N.id);
  area.innerHTML = markdownToNotes(blocked ?? (d.notes ? d.notes.text : ''));
  area.contentEditable = "true";
  notesStatus(blocked !== undefined ? t('meeting.notes.tooLong') : d.notes && d.notes.updatedAt ? t('meeting.notes.saved') : '', blocked !== undefined);
}

function notesStatus(text, error = false) {
  const el = $('.meeting .mt-notes-status');
  if (!el) return;
  el.textContent = text;
  el.classList.toggle('error', error);
}

function notesTyped() {
  N.blocked.delete(N.id);
  N.dirty = true;
  notesStatus('');
  clearTimeout(N.timer);
  N.timer = setTimeout(saveNotes, NOTES_WAIT);
}

// Notes from main: another save, or what main kept when the view opened. Text being written stays.
function notesArrived(notes) {
  if (!M.data) return;
  M.data.notes = notes;
  const area = $('.meeting .mt-notes-input');
  if (!area || N.id !== M.data.meeting.id || N.dirty || N.saving || N.blocked.has(N.id) || document.activeElement === area) return;
  if (notes.rev !== N.rev) {
    N.rev = notes.rev;
    area.innerHTML = markdownToNotes(notes.text);
  }
}

async function saveNotes() {
  clearTimeout(N.timer);
  if (!N.dirty || !N.id) return;
  const area = $('.meeting .mt-notes-input');
  if (!area || $('.meeting')?.dataset.id !== N.id) return;
  N.dirty = false;
  N.queued.set(N.id, { text: notesToMarkdown(area), rev: N.rev });
  if (!N.saving) await drainNotes();
}

// Saves the queued texts one at a time. A conflict is a save of this editor's own that crossed this one, because only
// this editor writes a meeting's notes: it saves over it with the revision main has now.
async function drainNotes() {
  N.saving = true;
  try {
    while (N.queued.size) {
      const [id, job] = N.queued.entries().next().value;
      N.queued.delete(id);
      if (N.id === id) notesStatus(t('meeting.notes.saving'));
      try {
        let res = await api('meetingSaveNotes', id, job.text, job.rev);
        if (res && res.conflict) res = await api('meetingSaveNotes', id, job.text, res.rev);
        if (res && res.tooLong) {
          // Keep this draft in memory for returning to the meeting. A permanent limit
          // failure must not retry forever or hold up another meeting's notes.
          if (!N.queued.has(id)) N.blocked.set(id, job.text);
          if (N.id === id && !N.dirty && !N.queued.has(id)) notesStatus(t('meeting.notes.tooLong'), true);
          continue;
        }
        if (res && !res.conflict) {
          N.blocked.delete(id);
          // A newer text queued meanwhile goes out with the revision this save made.
          const next = N.queued.get(id);
          if (next) next.rev = res.rev;
          if (N.id === id) N.rev = res.rev;
          if (M.data && M.data.meeting.id === id) M.data.notes = { text: job.text, rev: res.rev, updatedAt: Date.now() };
        }
        if (N.id === id) notesStatus(N.dirty || N.queued.has(id) ? '' : t('meeting.notes.saved'));
      } catch (_) {
        if (M.deleted.has(id)) continue;
        // Kept for the next try, unless something newer for that meeting is queued already.
        if (!N.queued.has(id)) N.queued.set(id, job);
        if (N.id === id) notesStatus(t('meeting.notes.failed'), true);
        clearTimeout(N.timer);
        N.timer = setTimeout(retryNotes, NOTES_WAIT);
        return;
      }
    }
  } finally {
    N.saving = false;
  }
  if (N.dirty && N.id) N.timer = setTimeout(saveNotes, NOTES_WAIT);
}

function retryNotes() {
  if (N.saving) return;
  if (N.dirty) saveNotes();
  else if (N.queued.size) drainNotes();
}

// Leaving the meeting or the view: what was typed goes out now.
function flushNotes() {
  if (N.dirty) saveNotes();
}

// ---------- events in the view ----------

async function onClick(e) {
  const b = e.target.closest('[data-mt], [data-mt-tab]');
  if (!b || b.disabled) return;
  if (b.dataset.mtTab) {
    M.tab = b.dataset.mtTab;
    setSegmented(b.closest('.segmented'), M.tab);
    $('.meeting').dataset.tab = M.tab;
    return;
  }
  const d = M.data;
  const id = d && d.meeting.id;
  switch (b.dataset.mt) {
    case 'rename':
      return beginRename(id);
    case 'more':
      return meetingMenu(b, id);
    case 'record':
      return record();
    case 'record-menu':
      return recordMenu(b);
    case 'screen':
      return monitorMenu(b);
    case 'show-frames':
      M.showFrames = !M.showFrames;
      localStorage.setItem('rukoo.meeting.showFrames', M.showFrames ? '1' : '0');
      b.setAttribute('aria-pressed', String(M.showFrames));
      return renderTranscript(true);
    case 'frame-group': {
      const row = b.closest('.mt-frame');
      const id = row.dataset.id;
      if (M.groupOpen.has(id)) M.groupOpen.delete(id);
      else M.groupOpen.add(id);
      row.querySelector('.mt-frame-strip').hidden = !M.groupOpen.has(id);
      b.setAttribute('aria-expanded', String(M.groupOpen.has(id)));
      fillFrames();
      return;
    }
    case 'frame':
      return toggleFrame(b.closest('.mt-frame'));
    case 'jump': {
      const sc = scroller();
      if (sc) sc.scrollTo({ top: sc.scrollHeight, behavior: 'smooth' });
      b.hidden = true;
      return;
    }
    case 'speaker':
      return speakerMenu(b);
    case 'event':
      return eventMenu(b);
    case 'open-event':
      if (d && d.event && !d.event.missing) ctx.openCalendarEvent(d.event.id, d.event.start);
      return;
    case 'edit-save':
      return closeEditor(true);
    case 'edit-cancel':
      return closeEditor(false);
    case 'mark':
      return command(() => api('meetingMark', id, null), () => toast(t('meeting.actions.marked', { time: clock(elapsed(d.meeting) || 0) })));
    case 'pause':
      return command(() => api('meetingPause', id));
    case 'resume':
      return command(() => api('meetingResume', id));
    case 'stop':
      // Two steps, as on Voice's pill: a meeting can't be recorded again.
      if (M.confirmStop !== id) {
        M.confirmStop = id;
        renderHead();
        setTimeout(() => {
          if (M.confirmStop !== id) return;
          M.confirmStop = null;
          renderHead();
        }, STOP_CONFIRM_MS);
        return;
      }
      M.confirmStop = null;
      renderHead();
      return command(() => api('meetingStop', id));
  }
}

async function command(run, done = null) {
  try {
    await run();
    if (done) done();
  } catch (err) {
    toast(voiceError(err), 6000);
  }
}

// The transcript is one tab stop: arrows move between its lines, Enter corrects a passage. In the editor Enter saves
// and Escape cancels.
function onViewKey(e) {
  if (e.target.matches('.mt-title-input')) {
    if (e.key === 'Enter' || e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      finishRename(e.key === 'Enter');
    }
    return;
  }
  if (e.target.closest('button')) return;
  if (e.target.matches && e.target.matches('.mt-edit')) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      closeEditor(false);
    } else if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      closeEditor(true);
    }
    return;
  }
  const list = e.target.closest && e.target.closest('.mt-transcript');
  if (!list) return;
  const rows = [...list.children];
  if (!rows.length) return;
  const at = rows.indexOf(e.target.closest('.mt-row'));
  let next = null;
  if (e.key === 'ArrowDown') next = rows[Math.min(rows.length - 1, at + 1)];
  else if (e.key === 'ArrowUp') next = rows[Math.max(0, at < 0 ? rows.length - 1 : at - 1)];
  else if (e.key === 'Home') next = rows[0];
  else if (e.key === 'End') next = rows[rows.length - 1];
  else if ((e.key === 'Enter' || e.key === ' ') && at >= 0 && rows[at].classList.contains('mt-frame')) {
    // A frame opens and closes; its text can then be selected and copied.
    e.preventDefault();
    return toggleFrame(rows[at]);
  } else if ((e.key === 'Enter' || e.key === 'F2') && at >= 0) {
    e.preventDefault();
    return editPassage(rows[at]);
  }
  if (!next) return;
  e.preventDefault();
  next.focus();
  next.scrollIntoView({ block: 'nearest' });
}

// ---------- sidebar ----------

// Only once the window has its accounts: before that app.js has no sidebar to draw.
function redrawSide() {
  if (ctx && ctx.S && ctx.S.data && ctx.S.data.accounts.length && document.querySelector('.sidebar-main')) ctx.renderSidebar();
}

let sideTimer = null;
function renderSideSoon() {
  clearTimeout(sideTimer);
  sideTimer = setTimeout(redrawSide, 0);
}

// "Record a meeting", then the meetings by day. In the rail a meeting shows as its day of the month.
export function sidebarHtml() {
  const why = notReadyText();
  const off = why ? 'is-off' : '';
  const busy = M.starting ? 'disabled' : '';
  // The main part records with Voice's default; the arrow chooses what to record of the screen first.
  const record = `<div class="mt-record-split is-side"><button class="btn compose-btn mt-record-btn ${off}" data-mt-side="record" title="${esc(why || t('meeting.actions.record'))}" ${busy}>${icons.record}<span>${esc(t('meeting.actions.record'))}</span></button>${recordMoreHtml(`btn compose-btn ${off}`, 'data-mt-side="record-menu"', busy)}</div>`;
  let lastGroup = null;
  const rows = M.list
    .map((m) => {
      const group = Number.isFinite(m.startedAt) ? groupLabel(m.startedAt) : '';
      const head = group !== lastGroup ? `<div class="nav-section"><span>${esc(group || t('meeting.sidebar.undated'))}</span></div>` : '';
      lastGroup = group;
      const title = titleOf(m);
      const isLive = live(m) && m.id === M.voice.recording;
      const time = Number.isFinite(m.startedAt) ? hhmm(m.startedAt) : '';
      const day = Number.isFinite(m.startedAt) ? new Date(m.startedAt).getDate() : '·';
      const active = M.open && m.id === M.id;
      const label = isLive ? t('meeting.sidebar.recordingTitle', { title }) : time ? t('meeting.sidebar.itemTitle', { title, time }) : title;
      return `${head}<button class="nav-item mt-nav ${active ? 'active' : ''} ${isLive ? 'is-live' : ''} ${m.removed ? 'is-removed' : ''}" data-mt-open="${esc(m.id)}" title="${esc(label)}" aria-label="${esc(label)}"${active ? ' aria-current="true"' : ''}><span class="ic mdot">${isLive ? '<i class="mt-rec-dot"></i>' : icons.mic}<b>${esc(day)}</b></span><span class="label">${esc(title)}</span><span class="count">${esc(time)}</span></button>`;
    })
    .join('');
  const list = rows || `<p class="mt-side-empty">${esc(t('meeting.sidebar.empty'))}</p>`;
  return `${record}<nav class="nav mt-list" aria-label="${esc(t('meeting.sidebar.label'))}">${list}</nav>`;
}

// Handles a click in the sidebar; false when it isn't the meeting list's.
export function sidebarClick(btn) {
  if (btn.dataset.mtSide === 'record') {
    record();
    return true;
  }
  if (btn.dataset.mtSide === 'record-menu') {
    recordMenu(btn);
    return true;
  }
  if (btn.dataset.mtOpen) {
    open(btn.dataset.mtOpen);
    return true;
  }
  return false;
}

// The sidebar's status line in meeting mode says how Rukoo Voice is: {cls, text, title}, as syncStatus in app.js.
export function footStatus() {
  const v = M.voice;
  if (v.status === 'connected') {
    return { cls: v.recording ? 'rec' : 'ok', text: v.recording ? t('meeting.voice.recording') : t('meeting.voice.connected'), title: v.voiceVersion ? t('meeting.voice.version', { version: v.voiceVersion }) : '' };
  }
  if (v.status === 'connecting') return { cls: 'busy', text: t('meeting.voice.connectingShort'), title: '' };
  return { cls: '', text: t('meeting.voice.offShort'), title: notReadyText(v) || '' };
}

// ---------- the title bar's recording indicator ----------

let ticker = null;
function afterVoice() {
  renderIndicator();
  const on = Boolean(M.voice.recording);
  if (on && !ticker) ticker = setInterval(tick, 1000);
  if (!on && ticker) {
    clearInterval(ticker);
    ticker = null;
  }
}

// Once a second while a meeting records: the clocks.
function tick() {
  renderIndicator();
  const s = M.data && M.data.meeting;
  const el = $('.meeting .mt-elapsed');
  if (el && live(s)) el.textContent = clock(elapsed(s));
}

export function renderIndicator() {
  const b = document.querySelector('[data-meeting-indicator]');
  if (!b) return;
  const id = M.voice.recording;
  b.hidden = !id;
  if (!id) return;
  const s = summaryOf(id);
  const paused = Boolean(s && s.state === 'paused');
  const ms = elapsed(s);
  const title = s ? titleOf(s) : t('meeting.untitledNoTime');
  b.classList.toggle('is-paused', paused);
  b.classList.toggle('is-hidden', Boolean(M.voice.hidden));
  b.dataset.meetingIndicator = id;
  b.querySelector('.rec-time').textContent = ms === null ? t('meeting.indicator.short') : clock(ms);
  const label = t(paused ? 'meeting.indicator.paused' : 'meeting.indicator.recording', { title });
  b.title = M.voice.hidden ? `${label}\n${t('meeting.head.hiddenHelp')}` : label;
  b.setAttribute('aria-label', label);
}

export const indicatorHtml = () =>
  `<button class="rec-indicator" data-meeting-indicator="" hidden><span class="rec-dot" aria-hidden="true"></span><span class="rec-time"></span>${icons.eyeOff}</button>`;
