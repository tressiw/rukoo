// Mock of the app's side of the pill, for pill.html?mock=<state>. Not used inside the app.
//
// pill.html?mock=<state>&theme=light|dark&lang=en|nl&edge=bottom|top&backdrop=1&hit=1&dev=1
//   <state>: one of STATES below, or a timeline: dictation, meeting, cycle (every state in turn).
//   backdrop=1 draws a document behind the pill; hit=1 outlines where the app would put the stop and monitor windows.
//   In a meeting state with the monitor button, a click on it moves to the next demo monitor.

const MINUTE = 60_000;

const meeting = (patch = {}) => ({
  state: 'recording',
  atMs: 12 * MINUTE + 4000,
  screen: 'off',
  compact: false,
  confirmStop: false,
  stopEnabled: true,
  rukooConnected: true,
  ...patch
});

export const STATES = {
  arming: { phase: 'arming' },
  listening: { phase: 'listening' },
  'listening-sound': { phase: 'listening', level: true },
  'listening-edit': { phase: 'listening', activity: 'rewrite', level: true },
  'no-audio': { phase: 'listening', notice: 'pill.noAudio' },
  warning: { phase: 'warning', notice: 'pill.selectTextBox', level: true },
  transcribing: { phase: 'transcribing' },
  'loading-model': { phase: 'transcribing', notice: 'pill.loadingModel' },
  rewriting: { phase: 'rewriting', activity: 'rewrite' },
  inserted: { phase: 'inserted', words: 12 },
  'inserted-one': { phase: 'inserted', words: 1 },
  rewrote: { phase: 'rewrote', activity: 'rewrite' },
  'nothing-heard': { phase: 'nothingHeard' },
  'error-offline': { phase: 'error', notice: 'pill.error.offline' },
  'error-no-key': { phase: 'error', notice: 'pill.error.noKey' },
  'error-auth': { phase: 'error', notice: 'pill.error.auth' },
  'error-timeout': { phase: 'error', notice: 'pill.error.timeout' },
  'error-mic': { phase: 'error', notice: 'pill.error.mic' },
  'error-model': { phase: 'error', notice: 'pill.error.model' },
  'error-model-missing': { phase: 'error', notice: 'pill.error.modelMissing' },
  'error-blocked': { phase: 'error', notice: 'pill.error.blocked' },
  'error-select-text': { phase: 'error', activity: 'rewrite', notice: 'pill.error.selectText' },
  'error-selection-too-long': { phase: 'error', activity: 'rewrite', notice: 'pill.error.selectionTooLong' },
  'error-generic': { phase: 'error', notice: 'pill.error.generic' },
  'meeting-recording': { phase: 'meeting', meeting: meeting() },
  'meeting-compact': { phase: 'meeting', meeting: meeting({ compact: true }) },
  'meeting-long': { phase: 'meeting', meeting: meeting({ atMs: 62 * MINUTE + 31_000 }) },
  'meeting-paused': { phase: 'meeting', meeting: meeting({ state: 'paused', atMs: 47 * MINUTE + 12_000 }) },
  'meeting-confirm': { phase: 'meeting', meeting: meeting({ confirmStop: true, compact: true }) },
  'meeting-hover': { phase: 'meeting', meeting: meeting({ compact: true }), pointer: { hover: true, pressed: false } },
  'meeting-saving': { phase: 'meeting', meeting: meeting({ state: 'processing', stopEnabled: false }) },
  'meeting-failed': { phase: 'meeting', notice: 'pill.meetingFailed', meeting: meeting({ state: 'failed', stopEnabled: false }) },
  'meeting-mic-lost': { phase: 'meeting', notice: 'pill.micLost', meeting: meeting() },
  'meeting-mic-back': { phase: 'meeting', notice: 'pill.micBack', meeting: meeting() },
  'meeting-video-dropped': { phase: 'meeting', notice: 'pill.videoDropped.disk', meeting: meeting({ screen: 'video', screenSwitch: true }) },
  'meeting-video-dropped-encoder': { phase: 'meeting', notice: 'pill.videoDropped.encoder', meeting: meeting({ screen: 'video', screenSwitch: true }) },
  'meeting-video-dropped-device': { phase: 'meeting', notice: 'pill.videoDropped.device', meeting: meeting({ screen: 'frames', screenSwitch: true }) },
  'meeting-frames-dropped': { phase: 'meeting', notice: 'pill.framesDropped.disk', meeting: meeting({ screen: 'frames' }) },
  'meeting-screen-failed': { phase: 'meeting', notice: 'pill.screenFailed', meeting: meeting({ screen: 'frames' }) },
  'meeting-screen-permission': { phase: 'meeting', notice: 'pill.screenPermission', meeting: meeting({ screen: 'off' }) },
  'meeting-screen-moved': { phase: 'meeting', notice: 'pill.screenMoved', args: { name: 'DELL U2720Q' }, meeting: meeting({ screen: 'frames', screenSwitch: true }) },
  'meeting-screen-button': { phase: 'meeting', meeting: meeting({ screen: 'frames', screenSwitch: true }) },
  'meeting-screen-off': { phase: 'meeting', meeting: meeting({ screen: 'off', screenSwitch: true }) },
  'meeting-screen-change-failed': { phase: 'meeting', notice: 'pill.screenChangeFailed', meeting: meeting({ screen: 'off', screenSwitch: true }) },
  'meeting-screen-hover': { phase: 'meeting', meeting: meeting({ screen: 'video', screenSwitch: true, compact: true }), pointer: { hover: true, pressed: false, target: 'screen' } },
  'meeting-dictating': { phase: 'listening', level: true, meeting: meeting({ stopEnabled: false, compact: true }) },
  hidden: { phase: 'hidden', visible: false }
};

// A dictation as the app plays it: key down, first buffer, speech, key up, inserted, hidden.
const DICTATION = [
  [0, 'arming'],
  [60, 'listening'],
  [700, 'listening-sound'],
  [3200, 'transcribing'],
  [4300, 'inserted'],
  [5000, 'hidden'],
  [6200, null]
];

// A meeting: the label shows for 4 s, then the pill compacts; a stop click asks first, the second one saves.
const MEETING = [
  [0, 'meeting-recording'],
  [4000, 'meeting-compact'],
  [6000, 'meeting-confirm'],
  [7600, 'meeting-saving'],
  [9800, 'hidden'],
  [11000, null]
];

export function createMock(params) {
  const handlers = new Map();
  const emit = (name, payload) => (handlers.get(name) || []).forEach((handler) => handler(payload));
  const mode = params.get('mock') && params.get('mock') !== '1' ? params.get('mock') : 'listening';
  const base = {
    theme: params.get('theme') === 'light' ? 'light' : 'dark',
    locale: params.get('lang') === 'nl' ? 'nl' : 'en',
    edge: params.get('edge') === 'top' ? 'top' : 'bottom',
    dev: params.get('dev') === '1'
  };
  if (params.get('backdrop') === '1') document.documentElement.classList.add('backdrop');

  let seq = 0;
  let current = null;
  let levelOn = false;
  let confirmTimer = 0;

  function snapshotFor(name) {
    const state = STATES[name] || STATES.listening;
    levelOn = !!state.level;
    return {
      seq: ++seq,
      visible: state.visible !== false,
      phase: state.phase,
      activity: state.activity || 'dictate',
      notice: state.notice ? { key: state.notice, args: { ...(state.args || {}) } } : null,
      words: state.words ?? null,
      meeting: state.meeting ? { ...state.meeting } : null,
      ...base
    };
  }

  function show(name) {
    current = snapshotFor(name);
    emit('pill://snapshot', current);
    const pointer = STATES[name]?.pointer;
    if (pointer) setTimeout(() => emit('pill://pointer', pointer), 50);
  }

  function play(timeline) {
    const run = () => {
      for (const [at, name] of timeline) setTimeout(() => (name ? show(name) : run()), at);
    };
    run();
  }

  // A speech-like level: syllables at about 5 Hz under a slower phrase envelope, with short pauses.
  const started = performance.now();
  setInterval(() => {
    if (!levelOn) return;
    const s = (performance.now() - started) / 1000;
    const phrase = Math.max(0, Math.sin(s * 0.9) * 0.6 + 0.5);
    const syllable = 0.55 + 0.45 * Math.sin(s * 31) * Math.sin(s * 11.7);
    emit('pill://level', { level: Math.max(0, Math.min(1, phrase * syllable)) });
  }, 20);

  // The stop and monitor buttons take clicks only in mock mode; in the app native windows over them do.
  const targetOf = (e) => (e.target.closest?.('.stop') ? 'stop' : e.target.closest?.('.screen-btn') ? 'screen' : null);
  document.addEventListener('pointerover', (e) => targetOf(e) && emit('pill://pointer', { hover: true, pressed: false, target: targetOf(e) }));
  document.addEventListener('pointerout', (e) => targetOf(e) && emit('pill://pointer', { hover: false, pressed: false, target: targetOf(e) }));
  document.addEventListener('pointerdown', (e) => targetOf(e) && emit('pill://pointer', { hover: true, pressed: true, target: targetOf(e) }));

  // Native menu choices are tested in Rust. The mock records opening the menu without changing capture.
  const screenMenu = { opened: 0 };
  document.addEventListener('pointerup', (e) => {
    if (targetOf(e) !== 'screen' || !current?.meeting) return;
    emit('pill://pointer', { hover: true, pressed: false, target: 'screen' });
    screenMenu.opened += 1;
  });

  document.addEventListener('pointerup', (e) => {
    if (targetOf(e) !== 'stop' || !current?.meeting) return;
    emit('pill://pointer', { hover: true, pressed: false, target: 'stop' });
    clearTimeout(confirmTimer);
    if (!current.meeting.confirmStop) {
      current = { ...current, seq: ++seq, meeting: { ...current.meeting, confirmStop: true, compact: true } };
      emit('pill://snapshot', current);
      confirmTimer = setTimeout(() => {
        current = { ...current, seq: ++seq, meeting: { ...current.meeting, confirmStop: false } };
        emit('pill://snapshot', current);
      }, 3000);
    } else {
      show('meeting-saving');
      setTimeout(() => show('hidden'), 2200);
    }
  });

  // For dev/check.js: send any payload as if the app had sent it.
  window.__pillMock = { emit, screenMenu };

  const hits = {};
  function drawHit(layout) {
    if (params.get('hit') !== '1') return;
    for (const name of ['button', 'screenButton']) {
      hits[name] ||= document.body.appendChild(Object.assign(document.createElement('div'), { className: 'hit-debug' }));
      const b = layout[name];
      hits[name].hidden = !b;
      if (b) Object.assign(hits[name].style, { left: `${b.x}px`, top: `${b.y}px`, width: `${b.width}px`, height: `${b.height}px` });
    }
  }

  const commands = {
    pill_snapshot: () => current,
    pill_ack: () => null,
    pill_layout: ({ layout }) => {
      window.__pillLayout = layout;
      drawHit(layout);
      return null;
    }
  };

  // The first snapshot is ready before pill.js asks for it.
  if (mode === 'dictation') play(DICTATION);
  else if (mode === 'meeting') play(MEETING);
  else if (mode === 'cycle') {
    const names = Object.keys(STATES);
    play(names.map((name, i) => [i * 1800, name]).concat([[names.length * 1800, null]]));
  } else {
    current = snapshotFor(mode);
    const pointer = STATES[mode]?.pointer;
    if (pointer) setTimeout(() => emit('pill://pointer', pointer), 400);
  }

  return {
    invoke: async (command, args) => {
      if (!(command in commands)) throw new Error(`mock: unknown command ${command}`);
      return commands[command](args);
    },
    listen: async (name, handler) => {
      handlers.set(name, [...(handlers.get(name) || []), handler]);
      return () => handlers.set(name, (handlers.get(name) || []).filter((h) => h !== handler));
    }
  };
}
