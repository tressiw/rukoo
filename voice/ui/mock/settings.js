// Mock of the app's side of Settings, for settings.html?mock=1. Not used inside the app. Paths and versions are
// made up; no real profile is read.
//
// settings.html?mock=1&page=<page>&theme=light|dark&lang=en|nl&scenario=<scenario>&later=1&features=<list>&sync=<sync>&eventFirst=1
//   <scenario>: default (set up, imported, Orukeet loaded), fresh (first start), busy (download running, Ownkey on
//   the same keys, Rukoo outdated), cloud (cloud meeting transcription without consent), failed (errors), dev
//   (development copy).
//   later=1 turns every feature in state.features on; features=screen,speakers,sync turns on the ones listed.
//   speakers=ready|off|platform|cpu|runtime|runtimeFailed|model|modelDamaged|key|consent|mistral|stopped: Speaker labels
//   in that state, with the feature on.
//   <sync> turns the sync feature on and sets bucket sync: fresh (the default: nothing filled in), tested (filled in
//   and tested, still off), on (up to date), live (a meeting uploads), retrying (the bucket can't be reached), failing
//   (the bucket refuses, three files lost), paused (a new secret or field waits for Test connection, with files still
//   waiting), unavailable (the switch is on but the sync store didn't open), key (the encryption key can't be opened),
//   restore (two meetings to restore), restore-failed (a restore that stopped). 1 means on. Test connection fails for the bucket "fail" and finds another key in the bucket
//   "shared".
//   eventFirst=1: the first settings_state answers with the state from before a change whose event arrives first,
//   as the app can (for dev/check.js).
//   features=<comma list>: the features this Voice shows (screen, speakers, sync); later=1 turns on all of them.
//   noEncoder=1, noOcr=1, screenUnavailable=1: a computer without a hardware video encoder, without an OCR language,
//   or whose Windows can't capture the screen. Each turns on the screen feature too.

const MB = 1_000_000;
const MIN = 60_000;
const NOW = Date.UTC(2026, 9, 30, 15, 42);
// settings_sync_key's code in the mock is zeros, never a key: 11 groups of 5, as voice-sync writes them.
const ZERO_CODE = `RKV1-${Array(11).fill('00000').join('-')}`;

// Mistral's realtime models stream meetings.
const REALTIME = 'voxtral-mini-transcribe-realtime';
const REALTIME_MODEL = 'voxtral-mini-transcribe-realtime-3';

const PROVIDERS = {
  audio: [
    { id: 'openai', endpoints: ['https://api.openai.com/v1/audio/transcriptions'], streamingPrefix: null },
    { id: 'google', endpoints: ['https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent'], streamingPrefix: null },
    { id: 'mistral', endpoints: ['https://api.mistral.ai/v1/audio/transcriptions'], streamingPrefix: REALTIME },
    { id: 'custom', endpoints: ['http://localhost:1234/v1/audio/transcriptions'], streamingPrefix: null },
    { id: 'orukeet', endpoints: [], streamingPrefix: null }
  ],
  rewrite: [
    { id: 'openai', endpoints: ['https://api.openai.com/v1/responses', 'https://api.openai.com/v1/chat/completions'] },
    { id: 'anthropic', endpoints: ['https://api.anthropic.com/v1/messages'] },
    { id: 'google', endpoints: ['https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent'] },
    { id: 'mistral', endpoints: ['https://api.mistral.ai/v1/chat/completions'] },
    { id: 'ollama', endpoints: ['http://localhost:11434/api/chat', 'https://ollama.com/api/chat'] },
    { id: 'openrouter', endpoints: ['https://openrouter.ai/api/v1/chat/completions'] },
    { id: 'custom', endpoints: ['http://localhost:1234/v1/chat/completions'] }
  ]
};

const MODELS = {
  mistral: ['voxtral-mini-latest', 'voxtral-small-latest', REALTIME_MODEL, 'mistral-small-latest', 'mistral-medium-latest'],
  openai: ['gpt-4o-mini-transcribe', 'gpt-4o-transcribe', 'whisper-1', 'gpt-5.5-mini'],
  google: ['gemini-2.5-flash', 'gemini-2.5-pro'],
  anthropic: ['claude-haiku-4-5', 'claude-sonnet-4-5'],
  openrouter: ['mistralai/mistral-small', 'openai/gpt-4o-mini'],
  ollama: ['llama3.2:3b', 'qwen2.5:7b'],
  // An OpenAI-compatible proxy in front of Mistral lists Mistral's ids.
  custom: ['voxtral-mini-latest', REALTIME_MODEL]
};

// Ownkey's filler lists and protection table (text_cleanup.py), for a preview close to what the app returns.
const FILLERS = {
  en: ['uh', 'uhh', 'um', 'umm', 'uhm', 'erm', 'er', 'mhm'],
  nl: ['uh', 'uhm', 'eh', 'ehm', 'euh', 'euhm', 'hm'],
  de: ['äh', 'ähm', 'ehm', 'hm', 'mh'],
  fr: ['euh', 'heu', 'hum'],
  es: ['eh', 'em', 'ehm', 'este']
};
const PROTECTED = { en: [], nl: ['er', 'este'], de: ['um', 'er', 'este'], fr: ['este'], es: ['er'] };

function clean(text, { enabled, languages, custom }) {
  if (!enabled) return text;
  const protectedWords = new Set(languages.flatMap((code) => PROTECTED[code] || []));
  const words = new Set();
  for (const code of languages) for (const word of FILLERS[code] || []) if (!protectedWords.has(word)) words.add(word);
  for (const word of String(custom || '').split(/[,\n]/)) if (word.trim()) words.add(word.trim().toLowerCase());
  if (!words.size) return text;
  const alternatives = [...words].map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|');
  let out = text.replace(new RegExp(`(^|[\\s,])(?:${alternatives})(?=[\\s,.!?]|$)[,]?`, 'giu'), '$1');
  out = out.replace(/\s{2,}/g, ' ').replace(/\s+([,.!?])/g, '$1').replace(/^[\s,]+/, '').replace(/,\s*,/g, ',');
  return out.replace(/(^|[.!?]\s+)(\p{Ll})/gu, (_, lead, c) => lead + c.toUpperCase());
}

function baseState() {
  return {
    seq: 1,
    version: '0.1.0',
    devMode: false,
    platform: 'windows',
    features: { screen: false, speakers: false, sync: false },
    theme: 'dark',
    locale: 'en',
    config: {
      dictation: { hotkey: 'right alt', rewriteHotkey: 'right ctrl', pasteMode: true, readyChime: false, startAtLogin: true },
      transcription: { provider: 'orukeet', endpoint: '', model: 'orukeet-onnx-int8', language: 'auto', keySaved: false },
      local: { modelFolder: '', idleMinutes: 20 },
      dictionary: {
        vocabulary: ['Rukoo', 'Orukeet', 'Hermes', 'bvdm.ai', 'Keendoc'],
        corrections: [
          { from: 'rocco', to: 'Rukoo' },
          { from: 'own key', to: 'Ownkey' }
        ]
      },
      fillers: { enabled: true, languages: ['en', 'nl'], custom: '' },
      rewriting: {
        provider: 'mistral',
        endpoint: 'https://api.mistral.ai/v1/chat/completions',
        model: 'mistral-small-latest',
        keySaved: true,
        auto: false,
        tone: 'auto',
        formatting: true,
        instructions: ''
      },
      meetings: {
        provider: 'orukeet',
        endpoint: '',
        model: 'orukeet-onnx-int8',
        keySaved: false,
        consent: null,
        retention: 'days7',
        stopKey: 'off',
        screen: 'off',
        liveSpeakers: true
      },
      ui: { language: 'system', theme: 'system', pillEdge: 'bottom', hidePillFromCapture: true },
      updates: { check: false },
      storage: {
        sync: { enabled: false, service: 'upcloud', endpoint: '', region: '', bucket: '', prefix: '', accessKeyId: '', retention: 'keep', secretSaved: false, tested: false, secret: '' }
      }
    },
    providers: PROVIDERS,
    dictation: { status: 'ready' },
    rukoo: { state: 'connected', clientVersion: '2.2.0', protocol: 1, port: 49821, discoveryPath: 'C:\\Users\\you\\AppData\\Local\\Rukoo Voice\\bridge.json', behind: 0 },
    models: {
      orukeet: {
        id: 'orukeet',
        name: 'Orukeet',
        sizeBytes: 487 * MB,
        stage: 'installed',
        percent: null,
        location: 'ownkey',
        path: 'C:\\Users\\you\\AppData\\Local\\Ownkey\\models\\orukeet-onnx-int8',
        removable: false,
        active: 'loaded',
        error: null
      },
      nemotron: {
        id: 'nemotron',
        name: 'Nemotron 3 Diarization',
        sizeBytes: 107 * MB,
        stage: 'installed',
        percent: null,
        location: 'ownkey',
        path: 'C:\\Users\\you\\AppData\\Local\\Ownkey\\models\\nemotron-3-diarization',
        removable: false,
        active: null,
        error: null
      }
    },
    // Worked out from the rest on every state (speakersState below).
    speakers: null,
    storage: { folder: 'C:\\Users\\you\\AppData\\Local\\Rukoo Voice\\meetings', usedBytes: 2.4e9, freeBytes: 182e9 },
    screen: { available: true, hardwareEncoder: 'NVIDIA H.264 Encoder MFT', ocr: ['nl-NL', 'en-US'], monitors: 2 },
    updates: { state: 'idle', version: null },
    ownkey: {
      running: false,
      keys: ['right alt', 'right ctrl'],
      configFound: true,
      configPath: '%APPDATA%\\Ownkey\\config.json',
      lastImport: NOW - 3 * 86_400_000,
      models: [
        { id: 'orukeet', name: 'Orukeet', sizeBytes: 487 * MB },
        { id: 'nemotron', name: 'Nemotron', sizeBytes: 107 * MB }
      ],
      report: {
        hotkeys: { dictation: 'right alt', rewrite: 'right ctrl' },
        transcription: { provider: 'orukeet', model: 'orukeet-onnx-int8', keyImported: false },
        rewriting: { provider: 'mistral', model: 'mistral-small-latest', keyImported: true },
        dictionary: { words: 5, corrections: 2 },
        fillers: { enabled: true, languages: ['en', 'nl'] },
        meetings: { provider: 'orukeet', keyImported: false, retention: 'days7', consent: false },
        skipped: ['summaries']
      }
    },
    sync: {
      available: false,
      state: 'off',
      waiting: 0,
      waitingBytes: 0,
      oldestWaiting: null,
      lastUpload: null,
      lastError: null,
      failingSince: null,
      lost: 0,
      screenRecording: false,
      key: { exists: false, id: null, exported: false },
      restore: { state: 'idle', available: 0, done: 0, total: 0, error: null },
      otherKeys: 0
    }
  };
}

// Made-up bucket details; nothing here is a real bucket or key.
const FILLED = {
  service: 'upcloud',
  endpoint: 'https://example.upcloudobjects.com',
  region: 'europe-1',
  bucket: 'my-meetings',
  prefix: 'rukoo-voice',
  accessKeyId: 'example-access-key',
  secretSaved: true,
  tested: true
};

// Each returns whether this computer has uploaded with its own key, which refuses another key (API.md).
const SYNC_SCENARIOS = {
  fresh: () => false,
  tested: (s) => {
    Object.assign(s.config.storage.sync, FILLED);
    return false;
  },
  on: (s) => {
    Object.assign(s.config.storage.sync, FILLED, { enabled: true });
    Object.assign(s.sync, { state: 'idle', lastUpload: NOW - 4 * MIN, key: { exists: true, id: '0f1e2d3c', exported: true } });
    return true;
  },
  live: (s) => {
    SYNC_SCENARIOS.on(s);
    Object.assign(s.sync, { state: 'live', waiting: 3, waitingBytes: 2.9 * MB, oldestWaiting: NOW - 40_000, lastUpload: NOW - 20_000 });
    return true;
  },
  retrying: (s) => {
    SYNC_SCENARIOS.on(s);
    Object.assign(s.sync, {
      state: 'retrying',
      waiting: 14,
      waitingBytes: 13.4 * MB,
      oldestWaiting: NOW - 25 * MIN,
      lastUpload: NOW - 26 * MIN,
      failingSince: NOW - 25 * MIN,
      lastError: { at: NOW - 30_000, code: 'network', message: 'Connection refused (os error 10061) connecting to example.upcloudobjects.com:443.' },
      key: { exists: true, id: '0f1e2d3c', exported: false }
    });
    return true;
  },
  failing: (s) => {
    SYNC_SCENARIOS.on(s);
    Object.assign(s.sync, {
      state: 'failing',
      waiting: 41,
      waitingBytes: 39.2 * MB,
      oldestWaiting: NOW - 26 * 60 * MIN,
      lastUpload: NOW - 26 * 60 * MIN,
      failingSince: NOW - 26 * 60 * MIN,
      lastError: {
        at: NOW - 2 * MIN,
        code: 'auth',
        message: 'HTTP 403 from example.upcloudobjects.com: SignatureDoesNotMatch. The request signature we calculated does not match the signature you provided.'
      },
      lost: 3
    });
    return true;
  },
  paused: (s) => {
    SYNC_SCENARIOS.failing(s);
    s.config.storage.sync.tested = false;
    Object.assign(s.sync, { state: 'paused', lost: 0 });
    return true;
  },
  // The sync store didn't open while the switch is on: the engine leaves sync out of features and says why.
  unavailable: (s) => {
    SYNC_SCENARIOS.on(s);
    s.features.sync = false;
    Object.assign(s.sync, {
      available: false,
      state: 'paused',
      lastUpload: null,
      lastError: { at: NOW - 3 * MIN, code: 'failed', message: 'queue.db has schema version 2, from a newer Voice' },
      key: { exists: false, id: null, exported: false }
    });
    return false;
  },
  key: (s) => {
    SYNC_SCENARIOS.on(s);
    Object.assign(s.sync, { state: 'key', key: { exists: true, id: '0f1e2d3c', exported: false } });
    return false;
  },
  restore: (s) => {
    SYNC_SCENARIOS.on(s);
    Object.assign(s.sync, { lastUpload: null, restore: { state: 'available', available: 2, done: 0, total: 0, error: null } });
    return false;
  },
  'restore-failed': (s) => {
    SYNC_SCENARIOS.restore(s);
    s.sync.restore = { state: 'failed', available: 1, done: 1, total: 2, error: 'The disk has less room than Voice keeps free, so it stopped before "Weekly".' };
    return false;
  }
};
SYNC_SCENARIOS['1'] = SYNC_SCENARIOS.on;

const SYNC_FIELDS = ['service', 'endpoint', 'region', 'bucket', 'prefix', 'accessKeyId'];
const LOOPBACK = /^http:\/\/(127\.0\.0\.1|localhost|\[::1\])(:\d+)?(\/|$)/;

const SCENARIOS = {
  default: () => {},
  fresh: (s) => {
    s.config.transcription = { provider: 'orukeet', endpoint: '', model: 'orukeet-onnx-int8', language: 'auto', keySaved: false };
    s.config.dictionary = { vocabulary: [], corrections: [] };
    s.config.rewriting.keySaved = false;
    s.dictation.status = 'modelMissing';
    s.rukoo = { ...s.rukoo, state: 'notRunning', clientVersion: null, protocol: null, behind: null };
    Object.assign(s.models.orukeet, { stage: 'missing', location: null, path: null, active: null });
    s.ownkey = { ...s.ownkey, lastImport: null, report: null, models: [] };
  },
  busy: (s) => {
    s.ownkey.running = true;
    s.dictation.status = 'loadingModel';
    s.rukoo = { ...s.rukoo, state: 'rukooOutdated', clientVersion: '2.1.2', protocol: null };
    s.config.local.modelFolder = 'D:\\Models\\Voice';
    Object.assign(s.models.orukeet, { stage: 'downloading', percent: 42, location: 'folder', path: 'D:\\Models\\Voice\\orukeet-onnx-int8', active: null, removable: false });
    s.updates = { state: 'available', version: '0.2.0' };
  },
  cloud: (s) => {
    s.config.transcription = { provider: 'openai', endpoint: 'https://api.openai.com/v1/audio/transcriptions', model: 'gpt-4o-mini-transcribe', language: 'nl', keySaved: true };
    s.config.meetings = { ...s.config.meetings, provider: 'mistral', endpoint: 'https://api.mistral.ai/v1/audio/transcriptions', model: 'voxtral-mini-latest', keySaved: true, consent: null };
    s.config.rewriting.auto = true;
    Object.assign(s.models.orukeet, { active: null });
    s.ownkey.report.meetings = { provider: 'mistral', keyImported: true, retention: 'keep', consent: false };
    s.ownkey.report.transcription = { provider: 'openai', model: 'gpt-4o-mini-transcribe', keyImported: true };
  },
  failed: (s) => {
    s.dictation.status = 'noKey';
    s.rukoo = { ...s.rukoo, state: 'refused', clientVersion: null, protocol: null, behind: null };
    s.config.local.modelFolder = '';
    Object.assign(s.models.orukeet, {
      stage: 'failed',
      percent: null,
      location: null,
      path: null,
      active: null,
      error: 'Checksum mismatch for encoder.int8.onnx: expected 7f3a…c01e, got 0b9d…44a2. The download may be damaged; try again.'
    });
    s.updates = { state: 'failed', version: null };
  },
  dev: (s) => {
    s.devMode = true;
    s.version = '0.1.0-dev';
    s.config.dictation = { ...s.config.dictation, hotkey: 'f13', rewriteHotkey: 'f14', startAtLogin: false };
    s.rukoo.discoveryPath = 'E:\\Development\\rukoo\\tmp\\voice-dev\\bridge.json';
    s.storage.folder = 'E:\\Development\\rukoo\\tmp\\voice-dev\\meetings';
    s.ownkey = { ...s.ownkey, lastImport: null, report: null };
  }
};

// Speaker labels, ?speakers=<name>. `forced` is what the engine would find wrong with this computer.
const MISTRAL_MEETINGS = { provider: 'mistral', endpoint: 'https://api.mistral.ai/v1/audio/transcriptions', model: REALTIME_MODEL, keySaved: true };
const SPEAKER_SCENARIOS = {
  ready: () => {},
  off: (s) => {
    s.config.meetings.liveSpeakers = false;
  },
  platform: (s, forced) => {
    s.platform = 'linux';
    forced.reason = 'platform';
  },
  cpu: (s, forced) => {
    Object.assign(forced, { reason: 'cpu', missing: ['bmi2'], cpu: { avx2: true, fma: true, f16c: true, bmi2: false } });
  },
  runtime: (s, forced) => {
    forced.reason = 'runtime';
  },
  runtimeFailed: (s, forced) => {
    forced.reason = 'runtimeFailed';
    forced.detail = 'LoadLibraryExW failed for C:\\Users\\you\\AppData\\Local\\Programs\\Rukoo Voice\\nemo_speech\\nemo_speech_asr_c.dll: The specified module could not be found. (os error 126)';
  },
  model: (s) => {
    Object.assign(s.models.nemotron, { stage: 'missing', location: null, path: null, removable: false });
  },
  modelDamaged: (s, forced) => {
    forced.reason = 'modelDamaged';
    forced.detail = 'Checksum mismatch for Nemotron-3-Diarization.q8_0.gguf: expected 08456d9e…c7a3a1, got 5e01…9b2c. Remove the model and download it again.';
  },
  key: (s) => {
    Object.assign(s.config.meetings, MISTRAL_MEETINGS, { keySaved: false, consent: { provider: 'mistral', host: 'api.mistral.ai', mode: 'stream' } });
  },
  consent: (s) => {
    Object.assign(s.config.meetings, MISTRAL_MEETINGS, { consent: null });
  },
  mistral: (s) => {
    Object.assign(s.config.meetings, MISTRAL_MEETINGS, { consent: { provider: 'mistral', host: 'api.mistral.ai', mode: 'stream' } });
  },
  stopped: (s, forced) => {
    forced.live = [{ meeting: 'm-20261030-171203-4f2a', mode: 'nemotron', state: 'failed', speakers: 3, reason: null, workerPid: null }];
  }
};

// SettingsState.speakers as the engine works it out (API.md).
function speakersState(state, forced) {
  const m = state.config.meetings;
  const host = (() => {
    try {
      return new URL(m.endpoint).hostname.toLowerCase();
    } catch {
      return '';
    }
  })();
  const mistral = m.provider === 'mistral' && String(m.model).startsWith(REALTIME);
  const installed = state.models.nemotron.stage === 'installed';
  const streamConsent = m.consent?.provider === 'mistral' && m.consent.host === host && m.consent.mode === 'stream';
  let reason = null;
  if (!m.liveSpeakers) reason = 'off';
  // The stream needs the meeting key before the agreement matters.
  else if (mistral) reason = !m.keySaved ? 'key' : streamConsent ? null : 'consent';
  else if (forced.reason) reason = forced.reason;
  else if (!installed) reason = 'model';
  return {
    engine: mistral ? 'mistral' : 'nemotron',
    ready: reason === null,
    reason,
    detail: reason && reason === forced.reason ? forced.detail || null : null,
    missing: reason === 'cpu' ? forced.missing || [] : [],
    cpu: forced.cpu || { avx2: true, fma: true, f16c: true, bmi2: true, neon: false, dotprod: false, fp16: false },
    runtime: !forced.reason && installed ? { location: 'voice', commit: '97a15af' } : null,
    live: forced.live || []
  };
}

export function createMock(params) {
  const state = baseState();
  (SCENARIOS[params.get('scenario')] || SCENARIOS.default)(state);
  // state.features as the engine builds it: every part with later=1, else those listed; a scenario parameter of
  // a feature (screen's, &speakers=, &sync=) turns that feature on too.
  const later = params.get('later') === '1';
  const listed = (params.get('features') || '').split(',').filter(Boolean);
  for (const name of Object.keys(state.features)) state.features[name] = later || listed.includes(name);
  if (params.get('noEncoder') === '1') state.screen.hardwareEncoder = null;
  if (params.get('noOcr') === '1') state.screen.ocr = [];
  if (params.get('screenUnavailable') === '1') state.screen = { available: false, hardwareEncoder: null, ocr: [], monitors: 2 };
  if (['noEncoder', 'noOcr', 'screenUnavailable'].some((name) => params.get(name) === '1')) state.features.screen = true;
  const forcedSpeakers = {};
  const speakers = params.get('speakers');
  if (speakers) {
    (SPEAKER_SCENARIOS[speakers] || SPEAKER_SCENARIOS.ready)(state, forcedSpeakers);
    state.features.speakers = true;
  }
  state.speakers = speakersState(state, forcedSpeakers);
  if (params.has('sync')) state.features.sync = true;
  state.sync.available = state.features.sync;
  let ownUploads = params.has('sync') ? (SYNC_SCENARIOS[params.get('sync')] || SYNC_SCENARIOS.fresh)(state) : false;
  const forced = { theme: params.get('theme'), locale: params.get('lang') };
  const handlers = [];
  let downloadTimer = 0;
  let syncTimer = 0;
  let protectedWindow = false;
  let eventFirst = params.get('eventFirst') === '1';
  // For dev/check.js: every command the page sent, with its arguments, as the app would receive them.
  const calls = [];
  window.__settingsMockCalls = calls;
  // For dev/check.js: commands that answer only after this many ms, as the app's do after a round trip.
  const slow = {};
  window.__settingsMockSlow = slow;
  window.__settingsMockProtected = () => protectedWindow;

  const resolve = () => {
    const ui = state.config.ui;
    state.theme = ui.theme === 'system' ? (forced.theme === 'light' ? 'light' : forced.theme === 'dark' ? 'dark' : matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark') : ui.theme;
    state.locale = ui.language === 'system' ? (forced.locale === 'nl' ? 'nl' : forced.locale === 'en' ? 'en' : navigator.language.startsWith('nl') ? 'nl' : 'en') : ui.language;
  };
  resolve();

  const snapshot = () => {
    state.speakers = speakersState(state, forcedSpeakers);
    return structuredClone(state);
  };
  // For dev/check.js: what the app would hold now.
  window.__settingsMockState = snapshot;
  const emit = () => {
    state.seq += 1;
    const copy = snapshot();
    handlers.forEach((handler) => handler(copy));
  };
  // For dev/check.js: sends a state exactly as given, seq included, as a misbehaving app would.
  window.__settingsMockSend = (next) => handlers.forEach((handler) => handler(structuredClone(next)));
  // For dev/check.js: changes what the app holds, such as a meeting that starts recording the screen, and sends the
  // new state unless { emit: false }, which leaves the page behind the app, as a race would.
  window.__settingsMockEdit = (change, { emit: send = true } = {}) => {
    change(state);
    if (send) emit();
  };
  const wait = (ms) => new Promise((done) => setTimeout(done, ms));
  // Tauri rejects a failed command with the command's error value; API.md defines it as { code, message }.
  const fail = (code, message) => {
    throw { code, message };
  };
  const setPath = (key, value) => {
    const parts = key.split('.');
    const last = parts.pop();
    parts.reduce((o, p) => o[p], state.config)[last] = value;
  };
  const getPath = (key) => key.split('.').reduce((o, p) => o?.[p], state.config);
  const host = (url) => {
    try {
      return new URL(url).hostname.toLowerCase();
    } catch {
      return '';
    }
  };

  // Sync turned on uploads what is on this computer, newest first, then is up to date.
  function syncOn(on) {
    clearInterval(syncTimer);
    if (!on) {
      state.sync.state = 'off';
      return;
    }
    if (!state.sync.key.exists) state.sync.key = { exists: true, id: '00000000', exported: false };
    Object.assign(state.sync, { state: 'uploading', waiting: 5, waitingBytes: 4.8 * MB, oldestWaiting: Date.now() });
    syncTimer = setInterval(() => {
      if (!state.config.storage.sync.enabled) return clearInterval(syncTimer);
      ownUploads = true;
      state.sync.waiting -= 1;
      state.sync.waitingBytes = state.sync.waiting * 0.96 * MB;
      state.sync.lastUpload = Date.now();
      if (state.sync.waiting <= 0) {
        Object.assign(state.sync, { state: 'idle', waiting: 0, waitingBytes: 0, oldestWaiting: null });
        clearInterval(syncTimer);
      }
      emit();
    }, 400);
  }

  // What the engine does when the connection changes: uploads pause until Test connection passes again. The switch
  // stays on, and so does retention's hold.
  function syncUntested() {
    state.config.storage.sync.tested = false;
    if (state.config.storage.sync.enabled) {
      clearInterval(syncTimer);
      state.sync.state = 'paused';
    }
  }

  function download(id) {
    const m = state.models[id];
    Object.assign(m, { stage: 'downloading', percent: 0, error: null });
    emit();
    clearInterval(downloadTimer);
    downloadTimer = setInterval(() => {
      if (m.stage === 'downloading') m.percent = Math.min(100, m.percent + 7);
      if (m.stage === 'downloading' && m.percent >= 100) Object.assign(m, { stage: 'verifying', percent: null });
      else if (m.stage === 'verifying') Object.assign(m, { stage: 'extracting', percent: null });
      else if (m.stage === 'extracting') {
        const folder = id === 'nemotron' ? 'nemotron-3-diarization' : 'orukeet-onnx-int8';
        Object.assign(m, { stage: 'installed', percent: null, location: 'voice', removable: true, path: `C:\\Users\\you\\AppData\\Local\\Rukoo Voice\\models\\${folder}` });
        if (id === 'orukeet' && state.config.transcription.provider === 'orukeet') {
          m.active = 'unloaded';
          state.dictation.status = 'ready';
        }
        clearInterval(downloadTimer);
      }
      emit();
    }, 300);
  }

  const commands = {
    settings_state: async () => {
      const answer = snapshot();
      if (eventFirst) {
        eventFirst = false;
        state.config.dictation.readyChime = !state.config.dictation.readyChime;
        emit();
        await wait(50);
      }
      return answer;
    },
    settings_set: ({ key, value }) => {
      const roles = ['dictation.hotkey', 'dictation.rewriteHotkey', 'meetings.stopKey'];
      if (roles.includes(key) && value !== 'off' && roles.some((role) => role !== key && getPath(role) === value)) fail('same_key', `${value} is already in use`);
      if (key === 'dictation.startAtLogin' && state.devMode) fail('dev_mode', 'Autostart is never registered in development mode.');
      if (key.startsWith('storage.sync.')) return setSync(key.slice('storage.sync.'.length), value);
      setPath(key, value);
      if (key.startsWith('ui.')) resolve();
      emit();
      return null;
    },
    settings_save_provider: async ({ save }) => {
      window.__lastSave = structuredClone(save);
      await wait(save.provider === 'orukeet' ? 1200 : 500);
      if (save.provider !== 'orukeet' && (!save.endpoint || !save.model)) fail('need_endpoint_model', 'Choose an endpoint and model before saving.');
      if (save.provider !== 'orukeet' && save.model === 'fail') fail('failed', 'HTTP 401 from api.mistral.ai: Unauthorized. Check the API key.');
      if (save.target !== 'meetings' && String(save.model).startsWith(REALTIME)) fail('invalid_value', 'Realtime models work for meeting transcription only.');
      const target = state.config[save.target];
      const sameProvider = target.provider === save.provider;
      Object.assign(target, { provider: save.provider, endpoint: save.endpoint, model: save.model });
      target.keySaved = save.apiKey === null ? sameProvider && target.keySaved : Boolean(save.apiKey);
      if (save.language) target.language = save.language;
      if (save.target === 'meetings') {
        const mode = String(save.model).startsWith(REALTIME) ? 'stream' : 'windows';
        target.consent = save.consent ? { provider: save.provider, host: host(save.endpoint), mode } : null;
      }
      if (save.target === 'transcription') {
        state.models.orukeet.active = save.provider === 'orukeet' && state.models.orukeet.stage === 'installed' ? 'loaded' : null;
        state.dictation.status =
          save.provider === 'orukeet' ? (state.models.orukeet.stage === 'installed' ? 'ready' : 'modelMissing') : target.keySaved ? 'ready' : 'noKey';
      }
      emit();
      return null;
    },
    settings_remove_key: ({ target }) => {
      if (target === 'sync') {
        state.config.storage.sync.secretSaved = false;
        syncUntested();
      } else state.config[target].keySaved = false;
      emit();
      return null;
    },
    settings_list_models: async ({ request }) => {
      await wait(700);
      if (!request.endpoint) fail('config', 'Choose an endpoint first.');
      return MODELS[request.provider] || [];
    },
    settings_model: ({ model, action }) => {
      const m = state.models[model];
      if (action === 'download') download(model);
      if (action === 'cancel') {
        clearInterval(downloadTimer);
        Object.assign(m, { stage: 'missing', percent: null });
        emit();
      }
      if (action === 'remove') {
        if (!m.removable) fail('not_removable', 'This model belongs to Ownkey; remove it in Ownkey.');
        const labelling = (forcedSpeakers.live || []).some((l) => l.mode === 'nemotron' && !['finished', 'failed', 'off'].includes(l.state));
        if (model === 'nemotron' && labelling) fail('busy', 'A meeting is labelling speakers with this model.');
        Object.assign(m, { stage: 'missing', location: null, path: null, removable: false });
        emit();
      }
      return null;
    },
    settings_choose_folder: ({ purpose }) => {
      // As the engine: Voice 0.1 keeps meetings in its data folder.
      if (purpose !== 'models') fail('invalid_value', "Meetings are kept in Voice's data folder in this version.");
      state.config.local.modelFolder = 'D:\\Models\\Voice';
      emit();
      return 'D:\\Models\\Voice';
    },
    settings_open_folder: () => null,
    settings_open_link: () => null,
    settings_clean_preview: ({ preview }) => clean(preview.text, preview),
    settings_check_updates: async () => {
      await wait(900);
      state.updates = { state: 'upToDate', version: null };
      emit();
      return structuredClone(state.updates);
    },
    settings_new_token: async () => {
      await wait(300);
      return null;
    },
    settings_import_ownkey: async () => {
      if (state.devMode) fail('dev_mode', 'In development, the import runs only with --import-ownkey <copy>.');
      await wait(600);
      state.ownkey.lastImport = Date.now();
      state.ownkey.report ||= baseState().ownkey.report;
      emit();
      return structuredClone(state.ownkey.report);
    },
    settings_sync_test: async () => {
      await wait(600);
      const sync = state.config.storage.sync;
      if (!sync.secretSaved) fail('no_secret', 'No secret access key is saved for the bucket.');
      if (!sync.endpoint) fail('endpoint', 'The endpoint URL is empty.');
      if (!sync.bucket || sync.bucket === 'fail') fail('bucket', `HTTP 404 from ${host(sync.endpoint)}: NoSuchBucket. The specified bucket does not exist.`);
      const paused = sync.enabled && !sync.tested;
      sync.tested = true;
      state.sync.otherKeys = sync.bucket === 'shared' ? 1 : 0;
      if (paused) syncOn(true);
      emit();
      return { otherKeys: state.sync.otherKeys };
    },
    settings_sync_save_secret: ({ secret }) => {
      if (typeof secret !== 'string' || !secret || secret.length > 1024) fail('invalid_value', 'The secret is empty or longer than 1,024 characters.');
      state.config.storage.sync.secretSaved = true;
      syncUntested();
      emit();
      return null;
    },
    settings_sync_key: ({ action, code }) => {
      const key = state.sync.key;
      if ((action === 'show' || action === 'import') && state.sync.screenRecording) fail('recording_screen', 'A meeting records the screen.');
      if (action === 'show') {
        if (state.sync.state === 'key') fail('key', 'Windows could not open master-key.bin (DPAPI error 0x8009000B).');
        if (!key.exists) state.sync.key = { exists: true, id: '00000000', exported: false };
        emit();
        return { code: ZERO_CODE, id: state.sync.key.id };
      }
      if (action === 'saved') {
        key.exported = true;
        emit();
        return null;
      }
      if (action === 'import') {
        const letters = String(code || '').toUpperCase().replace(/[\s-]/g, '');
        if (!/^RKV1[0-9A-Z]{55}$/.test(letters)) fail('invalid_value', "That isn't a recovery code.");
        if (ownUploads) fail('key_in_use', 'This computer uploaded meetings with its own key.');
        state.sync.key = { exists: true, id: '0f1e2d3c', exported: true };
        if (state.sync.state === 'key') state.sync.state = 'idle';
        emit();
        return { id: state.sync.key.id };
      }
      return fail('invalid_value', `Unknown action ${action}.`);
    },
    settings_sync_key_copy: () => {
      if (!state.sync.key.exists) fail('not_found', 'There is no encryption key yet.');
      if (state.sync.state === 'key') fail('key', 'Windows could not open master-key.bin (DPAPI error 0x8009000B).');
      return null;
    },
    settings_protect: ({ on }) => {
      protectedWindow = Boolean(on);
      return null;
    },
    settings_sync_restore: ({ action }) => {
      if (!state.config.storage.sync.enabled || !state.config.storage.sync.tested) fail('off', 'Sync is off.');
      if (action === 'list') {
        return {
          meetings: [
            { id: 'm-20261020-093000-a1b2', title: 'Design review', startedAt: NOW - 10 * 1440 * MIN, duration: 2_700_000 },
            { id: 'm-20261023-140000-c3d4', title: 'Weekly', startedAt: NOW - 7 * 1440 * MIN, duration: 1_800_000 }
          ].slice(0, state.sync.restore.available)
        };
      }
      if (action !== 'start') fail('invalid_value', `Unknown action ${action}.`);
      const total = state.sync.restore.available;
      state.sync.restore = { state: 'restoring', available: total, done: 0, total, error: null };
      emit();
      const timer = setInterval(() => {
        const r = state.sync.restore;
        r.done += 1;
        if (r.done >= r.total) {
          state.sync.restore = { state: 'done', available: 0, done: r.total, total: r.total, error: null };
          clearInterval(timer);
        }
        emit();
      }, 500);
      return null;
    },
    settings_close: () => null
  };

  // settings_set for storage.sync.*, with the engine's rules (API.md).
  function setSync(field, value) {
    const sync = state.config.storage.sync;
    if (field === 'secret') fail('invalid_value', 'Save the secret with settings_sync_save_secret.');
    if (field === 'secretSaved' || field === 'tested') fail('invalid_value', `storage.sync.${field} is read-only.`);
    if (field === 'enabled' && value === true && !sync.tested) fail('needs_test', 'Test the connection before you turn sync on.');
    if (field === 'service' && !['upcloud', 'r2', 'other'].includes(value)) fail('invalid_value', `Unknown service ${value}.`);
    if (field === 'retention' && !['keep', 'days365', 'days90'].includes(value)) fail('invalid_value', `Unknown retention ${value}.`);
    if (field === 'endpoint' && value && !value.startsWith('https://') && !LOOPBACK.test(value)) fail('invalid_value', 'The endpoint must start with https://, or http:// to this computer.');
    if (SYNC_FIELDS.includes(field) && sync[field] !== value) syncUntested();
    sync[field] = value;
    if (field === 'enabled') syncOn(value === true);
    emit();
    return null;
  }

  return {
    invoke: async (command, args = {}) => {
      calls.push([command, structuredClone(args)]);
      if (!(command in commands)) throw { code: 'unknown_command', message: `mock: unknown command ${command}` };
      if (slow[command]) await wait(slow[command]);
      return commands[command](args);
    },
    listen: async (name, handler) => {
      if (name === 'settings://state') handlers.push(handler);
      return () => {};
    }
  };
}
