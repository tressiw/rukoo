// The pill renders the app's snapshots and nothing else: the Rust state machine decides the phase, the holds and
// when to hide (voice/ui/API.md). One snapshot is one complete state, so a stale piece of the previous dictation
// never flashes, which Ownkey's patches could do.

import { loadCatalogs, setLanguage, t } from './lib/i18n.js';
import { connect, invoke, listen } from './lib/bridge.js';
import { pillIcon } from './lib/icons.js';

const PHASES = new Set(['hidden', 'arming', 'listening', 'transcribing', 'rewriting', 'inserted', 'rewrote', 'nothingHeard', 'warning', 'error', 'meeting']);
const MEETING_STATES = new Set(['recording', 'paused', 'processing', 'done', 'failed']);
const ERROR_NOTICES = new Set([
  'pill.error.selectText',
  'pill.error.selectionTooLong',
  'pill.error.offline',
  'pill.error.noKey',
  'pill.error.auth',
  'pill.error.timeout',
  'pill.error.mic',
  'pill.error.model',
  'pill.error.modelMissing',
  'pill.error.blocked',
  'pill.error.generic'
]);
const NOTICES = new Set([
  ...ERROR_NOTICES,
  'pill.noAudio',
  'pill.loadingModel',
  'pill.selectTextBox',
  'pill.meetingFailed',
  'pill.micLost',
  'pill.micBack',
  'pill.videoDropped.disk',
  'pill.videoDropped.encoder',
  'pill.videoDropped.device',
  'pill.framesDropped.disk',
  'pill.screenFailed',
  'pill.screenPermission',
  'pill.screenChangeFailed',
  'pill.screenMoved'
]);
// The screen notices of a meeting, with the screen glyph's tone: orange when Voice stopped recording part of the
// screen, grey when it only moved to another monitor.
const SCREEN_NOTICES = {
  'pill.videoDropped.disk': 'orange',
  'pill.videoDropped.encoder': 'orange',
  'pill.videoDropped.device': 'orange',
  'pill.framesDropped.disk': 'orange',
  'pill.screenFailed': 'orange',
  'pill.screenPermission': 'orange',
  'pill.screenChangeFailed': 'orange',
  'pill.screenMoved': 'text2'
};
const SCREENS = new Set(['off', 'frames', 'video']);

const LABEL_MAX = 220;
const EXIT_MS = 140;
const LABEL_OUT_MS = 140;
const POLL_MS = 1000;
const STOP_PADDING = 20;

// Ownkey's waveform (overlay-ui brand-waveform.tsx): rest heights, phase offsets and the level filter.
const BAR_REST = [0.4, 0.75, 1, 0.65, 0.45];
const BAR_PHASES = [0.0, 2.1, 4.4, 1.3, 3.2];
const BOUNCE_DELAYS = [0.05, 0.2, 0, 0.3, 0.12];
const SOUND_LEVEL = 0.05;
const ATTACK_MS = 90;
const RELEASE_MS = 160;
// Brief dips under the threshold between words don't drop the bars back to breathing.
const QUIET_HOLD_MS = 250;
const REDUCED_STEP_MS = 250;

const stage = document.getElementById('stage');
const measurer = document.getElementById('measure');
const reduced = matchMedia('(prefers-reduced-motion: reduce)');

let snapshot = null;
let receivedAt = 0;
let pill = null;
let level = 0;
let lastLayout = '';
let layoutQueued = false;

// ---------- snapshots ----------

const isObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);

// Like Ownkey's isOverlayState: a payload that fails the shape check is ignored, never half-rendered.
function isSnapshot(s) {
  if (!isObject(s) || !Number.isInteger(s.seq) || typeof s.visible !== 'boolean' || !PHASES.has(s.phase)) return false;
  if (s.activity !== 'dictate' && s.activity !== 'rewrite') return false;
  if (s.edge !== 'bottom' && s.edge !== 'top') return false;
  if (s.theme !== 'light' && s.theme !== 'dark') return false;
  if (typeof s.locale !== 'string') return false;
  if (s.notice != null && !(isObject(s.notice) && typeof s.notice.key === 'string')) return false;
  if (s.words != null && !Number.isInteger(s.words)) return false;
  if (s.meeting != null) {
    const m = s.meeting;
    if (!isObject(m) || !MEETING_STATES.has(m.state) || !Number.isFinite(m.atMs)) return false;
    for (const flag of ['compact', 'confirmStop', 'stopEnabled', 'rukooConnected']) if (typeof m[flag] !== 'boolean') return false;
    // An app from before screen recording sends neither.
    if (m.screen != null && !SCREENS.has(m.screen)) return false;
    if (m.screenSwitch != null && typeof m.screenSwitch !== 'boolean') return false;
  }
  return true;
}

function apply(next) {
  if (next == null) return;
  if (!isSnapshot(next)) {
    console.warn('pill: ignored a snapshot with an unexpected shape', next);
    return;
  }
  if (snapshot && next.seq <= snapshot.seq) return;
  const themeChanged = !snapshot || snapshot.theme !== next.theme;
  snapshot = next;
  receivedAt = performance.now();
  if (themeChanged) applyTheme(next.theme);
  setLanguage(next.locale);
  stage.classList.toggle('edge-top', next.edge === 'top');
  render();
  // After the first frame that painted this snapshot; the app's watchdog repairs a window that never paints (#13).
  const { seq } = next;
  requestAnimationFrame(() => requestAnimationFrame(() => invoke('pill_ack', { seq }).catch(() => {})));
}

function applyTheme(theme) {
  const root = document.documentElement;
  root.classList.add('theme-switching');
  root.classList.toggle('light', theme === 'light');
  requestAnimationFrame(() => requestAnimationFrame(() => root.classList.remove('theme-switching')));
}

// ---------- what to show ----------

const bars = (motion, tone) => ({ kind: 'bars', motion, tone });
const glyph = (name, size, tone) => ({ kind: 'icon', name, size, tone });
const text = (value, tone = 'text', shimmer = false) => ({ text: value, tone, shimmer });

function noticeOf(s) {
  const key = s.notice?.key;
  return key && NOTICES.has(key) ? key : null;
}

// A notice's own values, such as the monitor's name in pill.screenMoved. They reach the page only as textContent.
function noticeArgs(s) {
  const args = isObject(s.notice?.args) ? s.notice.args : {};
  return Object.fromEntries(Object.entries(args).filter(([, value]) => typeof value === 'string' || Number.isFinite(value)));
}

function viewOf(s) {
  const notice = noticeOf(s);
  const view = { indicator: { kind: 'none' }, label: null, time: null, screen: null, stop: null, error: false, dev: s.dev === true };
  switch (s.phase) {
    case 'arming':
      view.indicator = bars('ripple', 'muted');
      view.label = text(t('pill.arming'));
      break;
    case 'listening':
      if (notice === 'pill.noAudio') {
        view.indicator = bars('breathe', 'orange');
        view.label = text(t('pill.noAudio'), 'muted');
      } else {
        view.indicator = bars('listen', 'orange');
        view.label = text(t(s.activity === 'rewrite' ? 'pill.listeningEdit' : 'pill.listening'));
      }
      break;
    case 'warning':
      view.indicator = glyph('info', 14, 'orange');
      view.label = text(t(notice && !ERROR_NOTICES.has(notice) ? notice : 'pill.selectTextBox'));
      break;
    case 'transcribing':
      view.indicator = bars('bounce', 'orange');
      view.label = text(t(notice === 'pill.loadingModel' ? 'pill.loadingModel' : 'pill.transcribing'), 'text', true);
      break;
    case 'rewriting':
      view.indicator = bars('bounce', 'orange');
      view.label = text(t('pill.rewriting'), 'text', true);
      break;
    case 'inserted':
      view.indicator = glyph('check', 16, 'green');
      view.label = text(t('pill.inserted', { count: s.words ?? 0 }));
      break;
    case 'rewrote':
      view.indicator = glyph('check', 16, 'green');
      view.label = text(t('pill.rewrote'));
      break;
    case 'nothingHeard':
      // Differs from Ownkey on purpose: an empty capture showed the green Done check although nothing was typed.
      view.label = text(t('pill.nothingHeard'), 'muted');
      break;
    case 'error':
      view.indicator = glyph('x', 14, 'red');
      view.label = text(t(ERROR_NOTICES.has(notice) ? notice : 'pill.error.generic'), 'error');
      view.error = true;
      break;
    case 'meeting':
      meetingView(s.meeting, notice, view, noticeArgs(s));
      break;
  }
  return view;
}

function meetingView(m, notice, view, args) {
  if (m.state === 'processing') {
    view.indicator = { kind: 'spinner' };
    view.label = text(t('pill.meeting.saving'));
    return;
  }
  if (m.state === 'failed') {
    view.indicator = glyph('x', 14, 'red');
    view.label = text(t('pill.meetingFailed'), 'error');
    view.error = true;
    return;
  }
  const paused = m.state === 'paused';
  view.time = { paused };
  view.stop = m.stopEnabled ? { confirm: m.confirmStop } : null;
  // Available even when capture is off, so an audio-only meeting can start screen recording.
  view.screen = m.screenSwitch === true ? {} : null;
  if (notice === 'pill.micLost') {
    view.indicator = glyph('mic', 14, 'orange');
    view.label = text(t('pill.micLost'));
  } else if (notice === 'pill.micBack') {
    view.indicator = glyph('mic', 14, 'green');
    view.label = text(t('pill.micBack'));
  } else if (SCREEN_NOTICES[notice]) {
    view.indicator = glyph('screen', 14, SCREEN_NOTICES[notice]);
    view.label = text(t(notice, notice === 'pill.screenMoved' ? { name: String(args.name ?? '') } : args));
  } else if (paused) {
    // Paused never compacts, so a paused meeting isn't forgotten.
    view.indicator = glyph('pause', 12, 'text2');
    view.label = text(t('pill.meeting.paused'));
  } else {
    view.indicator = { kind: 'dot' };
    view.label = m.compact ? null : text(t('pill.meeting.recording'));
  }
}

const shown = (s) => s.visible && s.phase !== 'hidden' && !(s.phase === 'meeting' && (!s.meeting || s.meeting.state === 'done'));

// ---------- rendering ----------

function el(tag, className) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  return node;
}

function render() {
  if (!shown(snapshot)) {
    hide();
    return;
  }
  // A new state during the exit wins: the leaving pill goes at once and a fresh one pops in.
  if (pill?.leaving) {
    pill.observer.disconnect();
    pill.root.remove();
    pill = null;
  }
  const first = !pill;
  if (first) pill = create();
  const view = viewOf(snapshot);
  pill.view = view;
  pill.root.classList.toggle('is-error', view.error);
  setIndicator(view.indicator);
  setLabel(view.label, first);
  setTime(view.time);
  pill.dev.hidden = !view.dev;
  pill.dev.textContent = t('pill.dev');
  setScreen(view.screen);
  setStop(view.stop, first);
  pill.root.classList.toggle('has-stop', !!view.stop);
  pill.root.classList.toggle('has-screen', !!view.screen);
  syncLevelLoop();
  queueLayout();
}

function create() {
  const root = el('div', 'pill');
  const ind = el('span', 'ind none');
  const slot = el('span', 'label-slot');
  slot.setAttribute('role', 'status');
  slot.setAttribute('aria-live', 'polite');
  const time = el('span', 'time');
  time.setAttribute('aria-hidden', 'true');
  time.hidden = true;
  const dev = el('span', 'dev');
  dev.hidden = true;
  // Like the stop button, a picture of a button: the app puts a native window over it for the clicks.
  const screen = el('span', 'screen-btn');
  screen.setAttribute('role', 'img');
  screen.innerHTML = pillIcon('monitor', 15);
  screen.hidden = true;
  const stop = el('span', 'stop');
  stop.setAttribute('role', 'img');
  stop.innerHTML = `<span class="stop-ic">${pillIcon('stop', 10)}</span>`;
  const stopLabel = el('span', 'stop-label');
  stop.append(stopLabel);
  stop.hidden = true;
  root.append(ind, slot, time, dev, screen, stop);
  root.addEventListener('animationend', queueLayout);
  root.addEventListener('transitionend', queueLayout);
  stage.replaceChildren(root);
  const observer = new ResizeObserver(queueLayout);
  observer.observe(root);
  observer.observe(stop);
  observer.observe(screen);
  return { root, ind, slot, time, dev, screen, stop, stopLabel, observer, indicatorKey: '', labelKey: null, leaving: false, timer: 0, view: null };
}

function hide() {
  if (!pill || pill.leaving) return;
  const current = pill;
  current.leaving = true;
  clearTimeout(current.timer);
  current.root.classList.add('leaving');
  queueLayout();
  syncLevelLoop();
  // Render nothing after the exit, so the next show starts from an empty frame (Ownkey's voice-overlay.tsx).
  setTimeout(() => {
    if (pill !== current) return;
    current.observer.disconnect();
    current.root.remove();
    pill = null;
  }, EXIT_MS);
}

function setIndicator(ind) {
  const key = JSON.stringify(ind);
  if (key === pill.indicatorKey) return;
  const before = pill.indicatorKey ? JSON.parse(pill.indicatorKey) : null;
  pill.indicatorKey = key;
  const node = pill.ind;
  node.className = `ind ${ind.kind === 'none' ? 'none' : ''} ${ind.tone ? `tone-${ind.tone}` : ''}`;
  node.style.setProperty('--w', `${ind.kind === 'bars' ? 24 : ind.kind === 'icon' ? ind.size : 12}px`);
  // Bars stay the same elements from Starting through Transcribing, so only their motion and color change.
  if (ind.kind === 'bars' && before?.kind === 'bars') {
    node.firstElementChild.className = `bars ${ind.motion}`;
    resetBars();
    return;
  }
  if (ind.kind === 'bars') {
    const group = el('span', `bars ${ind.motion}`);
    BAR_REST.forEach((rest, i) => {
      const bar = el('span', 'bar');
      bar.style.setProperty('--rest', String(rest));
      bar.style.setProperty('--i', String(i));
      bar.style.setProperty('--phase', String(BAR_PHASES[i]));
      bar.style.setProperty('--bounce', `${BOUNCE_DELAYS[i]}s`);
      group.append(bar);
    });
    node.replaceChildren(group);
  } else if (ind.kind === 'icon') {
    node.innerHTML = pillIcon(ind.name, ind.size);
    if (ind.name === 'check') node.firstElementChild.classList.add('check');
  } else if (ind.kind === 'dot') {
    node.replaceChildren(el('span', 'rec-dot'));
  } else if (ind.kind === 'spinner') {
    node.replaceChildren(el('span', 'spin'));
  } else {
    node.replaceChildren();
  }
}

function measure(value) {
  measurer.textContent = value;
  return Math.min(LABEL_MAX, Math.ceil(measurer.getBoundingClientRect().width) + 1);
}

function setLabel(label, first) {
  const key = label ? `${label.tone}|${label.shimmer}|${label.text}` : '';
  if (key === pill.labelKey) return;
  pill.labelKey = key;
  const slot = pill.slot;
  for (const old of slot.querySelectorAll('.label:not(.out)')) {
    if (first) {
      old.remove();
      continue;
    }
    old.classList.remove('in');
    old.classList.add('out');
    setTimeout(() => old.remove(), LABEL_OUT_MS);
  }
  const width = label ? measure(label.text) : 0;
  if (label) {
    const span = el('span', `label ${label.tone === 'text' ? '' : label.tone} ${label.shimmer ? 'shimmer' : ''} ${first ? '' : 'in'}`);
    span.textContent = label.text;
    span.style.width = `${width}px`;
    slot.append(span);
  }
  pill.root.classList.toggle('compact', !label);
  if (first) {
    // The first label of an appearance comes with the pill's own entry, at its final width.
    slot.style.transition = 'none';
    slot.style.width = `${width}px`;
    void slot.offsetWidth;
    slot.style.transition = '';
  } else {
    slot.style.width = `${width}px`;
  }
}

// Labels measured before Inter loaded come out short; measure again once it has (Ownkey 97fa785).
function remeasure() {
  if (!pill || !pill.view) return;
  const label = pill.slot.querySelector('.label:not(.out)');
  if (label && pill.view.label) {
    const width = measure(pill.view.label.text);
    label.style.width = `${width}px`;
    pill.slot.style.width = `${width}px`;
  }
  if (pill.view.stop?.confirm) pill.stop.style.width = `${measure(t('pill.meeting.confirmStop')) + STOP_PADDING}px`;
}

function setScreen(screen) {
  const node = pill.screen;
  node.hidden = !screen;
  node.setAttribute('aria-label', t('pill.screenButton'));
  if (!screen) node.classList.remove('hover', 'pressed');
}

function setStop(stop, first) {
  const node = pill.stop;
  node.hidden = !stop;
  node.setAttribute('aria-label', t('pill.stopButton'));
  pill.stopLabel.textContent = t('pill.meeting.confirmStop');
  if (!stop) {
    node.classList.remove('confirm', 'hover', 'pressed');
    return;
  }
  node.classList.toggle('confirm', stop.confirm);
  const width = stop.confirm ? measure(t('pill.meeting.confirmStop')) + STOP_PADDING : 28;
  if (first) node.style.transition = 'none';
  node.style.width = `${width}px`;
  if (first) {
    void node.offsetWidth;
    node.style.transition = '';
  }
}

// ---------- the meeting clock ----------

const pad = (n) => String(n).padStart(2, '0');

function clock(ms) {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

// The meeting clock keeps running while paused (PROTOCOL.md); the app resends the snapshot every minute, which
// corrects drift.
const elapsed = () => snapshot.meeting.atMs + (performance.now() - receivedAt);

function setTime(time) {
  const node = pill.time;
  clearTimeout(pill.timer);
  node.hidden = !time;
  if (!time) return;
  node.classList.toggle('paused', time.paused);
  const tick = () => {
    if (!pill || pill.time !== node || !pill.view?.time) return;
    const ms = elapsed();
    node.textContent = clock(ms);
    pill.timer = setTimeout(tick, 1000 - (ms % 1000) + 5);
  };
  tick();
}

// ---------- the level ----------

let loop = 0;
let stepTimer = 0;
let values = BAR_REST.map(() => 0.35);
let lastFrame = 0;
let quietSince = 0;

function listeningBars() {
  return pill && !pill.leaving ? pill.ind.querySelector('.bars.listen') : null;
}

function resetBars() {
  values = BAR_REST.map(() => 0.35);
  lastFrame = 0;
  for (const bar of pill.ind.querySelectorAll('.bar')) bar.style.transform = '';
  pill.ind.querySelector('.bars')?.classList.remove('sound');
}

function syncLevelLoop() {
  const live = !!listeningBars();
  if (!live) {
    cancelAnimationFrame(loop);
    clearInterval(stepTimer);
    loop = 0;
    stepTimer = 0;
    return;
  }
  if (reduced.matches) {
    cancelAnimationFrame(loop);
    loop = 0;
    if (!stepTimer) {
      stepTimer = setInterval(step, REDUCED_STEP_MS);
      step();
    }
  } else {
    clearInterval(stepTimer);
    stepTimer = 0;
    if (!loop) loop = requestAnimationFrame(frame);
  }
}

// Ownkey's level formula: eased = level^0.8, a small per-bar wobble, and a one-pole filter that rises in 90ms and
// falls in 160ms.
function frame(now) {
  loop = 0;
  const group = listeningBars();
  if (!group) return;
  const loud = level >= SOUND_LEVEL;
  if (loud) quietSince = 0;
  else if (!quietSince) quietSince = now;
  const sound = loud || now - quietSince < QUIET_HOLD_MS;
  if (sound !== group.classList.contains('sound')) {
    group.classList.toggle('sound', sound);
    if (sound) values = BAR_REST.map(() => 0.35);
    else for (const bar of group.children) bar.style.transform = '';
  }
  if (sound) {
    const dt = Math.min(64, lastFrame ? now - lastFrame : 16);
    const eased = Math.pow(Math.max(0, Math.min(1, level)), 0.8);
    const seconds = now / 1000;
    [...group.children].forEach((bar, i) => {
      const wobble = 1 + 0.14 * eased * Math.sin(seconds * 7.3 + BAR_PHASES[i]);
      const target = (0.35 + 0.65 * eased) * wobble;
      const tau = target > values[i] ? ATTACK_MS : RELEASE_MS;
      values[i] = Math.max(0.2, Math.min(1.12, values[i] + (target - values[i]) * (1 - Math.exp(-dt / tau))));
      bar.style.transform = `scaleY(${values[i].toFixed(3)})`;
    });
  }
  lastFrame = now;
  loop = requestAnimationFrame(frame);
}

// Reduced motion: three steps at most four times a second, without interpolation. Differs from Ownkey on purpose:
// it froze the bars, which hid whether the microphone heard anything.
function step() {
  const group = listeningBars();
  if (!group) return;
  group.classList.add('sound');
  const scale = level < SOUND_LEVEL ? 0.35 : level < 0.4 ? 0.6 : 1;
  for (const bar of group.children) bar.style.transform = `scaleY(${scale})`;
}

reduced.addEventListener('change', () => {
  if (pill) resetBars();
  syncLevelLoop();
});

// ---------- layout for the stop window ----------

function queueLayout() {
  if (layoutQueued) return;
  layoutQueued = true;
  requestAnimationFrame(() => {
    layoutQueued = false;
    reportLayout();
  });
}

function box(node) {
  const r = node.getBoundingClientRect();
  const x = Math.floor(r.left);
  const y = Math.floor(r.top);
  return { x, y, width: Math.ceil(r.right) - x, height: Math.ceil(r.bottom) - y };
}

// The app puts its native stop window over `button` and its monitor window over `screenButton`, in CSS pixels from
// the window's top left. null means that button isn't drawn, so its window hides.
function reportLayout() {
  const live = pill && !pill.leaving;
  const layout = {
    pill: live ? box(pill.root) : null,
    button: live && pill.view?.stop ? box(pill.stop) : null,
    screenButton: live && pill.view?.screen ? box(pill.screen) : null
  };
  const key = JSON.stringify(layout);
  if (key === lastLayout) return;
  lastLayout = key;
  invoke('pill_layout', { layout: { seq: snapshot?.seq ?? 0, ...layout } }).catch(() => {});
}

// ---------- start ----------

async function start() {
  await connect('pill');
  await loadCatalogs();
  measurer.style.fontFamily = getComputedStyle(document.body).fontFamily;
  document.fonts.load('500 12.5px Inter').then(remeasure, () => {});
  document.fonts.addEventListener('loadingdone', remeasure);
  await listen('pill://snapshot', apply);
  await listen('pill://level', (payload) => {
    if (payload && Number.isFinite(payload.level)) level = Math.max(0, Math.min(1, payload.level));
  });
  // target names the native window the pointer is over; an app from before the monitor button sends none.
  await listen('pill://pointer', (payload) => {
    if (!pill || !payload) return;
    const node = payload.target === 'screen' ? pill.screen : pill.stop;
    node.classList.toggle('hover', payload.hover === true);
    node.classList.toggle('pressed', payload.pressed === true);
  });
  apply(await invoke('pill_snapshot'));
  // A safety net for a missed event; the app's ack watchdog covers the paint itself.
  setInterval(() => invoke('pill_snapshot').then(apply, () => {}), POLL_MS);
}

start().catch((error) => console.error('pill: failed to start', error));
