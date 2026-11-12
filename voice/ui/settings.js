// Rukoo Voice Settings: Rukoo's Settings pattern, a single column of grouped cards where a row changes a value in
// place or opens a sub-page. The app owns every value; this page renders settings_state() and the
// settings://state events and sends changes back (voice/ui/API.md). Secrets never come back to this page: a saved API
// key or bucket secret is only a "Key saved" flag. The one exception is the encryption key's recovery code, which
// lives only in its own dialog while that is open.

import { loadCatalogs, setLanguage, language, t, tHtml, esc, localize, list, bytes, dateTime, languageName } from './lib/i18n.js';
import { connect, invoke, listen, params, mocked } from './lib/bridge.js';
import { icons } from './lib/icons.js';

// Parts of Settings that come with a feature: the engine lists them in state.features (voice/ui/API.md).
// ?later=1 shows every part in mock mode, for design review.
const LATER = mocked && params.get('later') === '1';
const feature = (name) => LATER || state?.features?.[name] === true;

const HOTKEYS = ['right alt', 'right ctrl', 'right shift', 'f13', 'f14', 'f15', 'pause', 'scroll lock'];
const KEY_NAMES = {
  'right alt': 'settings.keys.rightAlt',
  'right ctrl': 'settings.keys.rightCtrl',
  'right shift': 'settings.keys.rightShift',
  pause: 'settings.keys.pause',
  'scroll lock': 'settings.keys.scrollLock'
};
// The keys a choice can't share, with the hint that says who has it.
const KEY_ROLES = {
  'dictation.hotkey': 'settings.keys.usedDictation',
  'dictation.rewriteHotkey': 'settings.keys.usedRewrite',
  'meetings.stopKey': 'settings.keys.usedStop'
};
const KEY_TITLES = {
  'dictation.hotkey': 'settings.dictation.key',
  'dictation.rewriteHotkey': 'settings.rewriting.key',
  'meetings.stopKey': 'settings.meetings.stopKey'
};

const SPOKEN_LANGUAGES = ['auto', 'en', 'nl', 'de', 'fr', 'es', 'it', 'pt', 'pl', 'ja', 'zh'];
const UI_LANGUAGES = [
  ['en', 'English'],
  ['nl', 'Nederlands']
];
const THEMES = [
  ['system', 'common.system'],
  ['light', 'settings.theme.light'],
  ['dark', 'settings.theme.dark']
];
const EDGES = [
  ['bottom', 'settings.pill.bottom'],
  ['top', 'settings.pill.top']
];
const RETENTION = [
  ['days7', 'settings.retention.days7'],
  ['keep', 'settings.retention.keep'],
  ['after_transcription', 'settings.retention.afterTranscription']
];
const TONES = [
  ['auto', 'settings.tone.auto'],
  ['professional', 'settings.tone.professional'],
  ['casual', 'settings.tone.casual'],
  ['friendly', 'settings.tone.friendly'],
  ['concise', 'settings.tone.concise']
];
const SCREEN = [
  ['off', 'common.off'],
  ['frames', 'settings.meetings.screenFrames'],
  ['video', 'settings.meetings.screenVideo']
];
// How long the bucket keeps a meeting's media; the text stays (storage.sync.retention, API.md).
const SYNC_RETENTION = [
  ['keep', 'settings.sync.retentionKeep'],
  ['days365', 'settings.sync.retentionYear'],
  ['days90', 'settings.sync.retention90']
];
// Test connection's codes, also lastError.code, to their text. Written out in full: the i18n test can't see keys
// built at run time.
const SYNC_ERRORS = {
  endpoint: 'settings.sync.error.endpoint',
  network: 'settings.sync.error.network',
  auth: 'settings.sync.error.auth',
  bucket: 'settings.sync.error.bucket',
  clock: 'settings.sync.error.clock',
  permission: 'settings.sync.error.permission',
  no_secret: 'settings.sync.error.noSecret',
  failed: 'settings.sync.error.failed'
};
const KEY_ERRORS = {
  invalid_value: 'settings.sync.keyInvalid',
  key_in_use: 'settings.sync.keyInUse',
  recording_screen: 'settings.sync.keyScreen'
};
// A change to any of these pauses uploads until Test connection passes again; the switch stays (API.md).
const SYNC_FIELDS = ['service', 'endpoint', 'region', 'bucket', 'prefix', 'accessKeyId'];
const SYNC_ENDPOINTS = { upcloud: 'https://….upcloudobjects.com', r2: 'https://….r2.cloudflarestorage.com', other: 'https://' };

// Ownkey's filler lists (text_cleanup.py); the real cleanup runs in the app, these only describe each switch.
const FILLER_WORDS = {
  en: ['uh', 'uhh', 'um', 'umm', 'uhm', 'erm', 'er', 'mhm'],
  nl: ['uh', 'uhm', 'eh', 'ehm', 'euh', 'euhm', 'hm'],
  de: ['äh', 'ähm', 'ehm', 'hm', 'mh'],
  fr: ['euh', 'heu', 'hum'],
  es: ['eh', 'em', 'ehm', 'este']
};
const FILLER_SAMPLE = 'Um, I think, uh, we should ehm ship it on Tuesday. Er is nog één ding.';

const BRANDS = { openai: 'OpenAI', anthropic: 'Anthropic', google: 'Google', mistral: 'Mistral', ollama: 'Ollama', openrouter: 'OpenRouter' };
const ORUKEET_MODEL = 'orukeet-onnx-int8';
// Ownkey never asks before sending to these hosts: they are this computer.
const LOCAL_HOSTS = new Set(['localhost', '127.0.0.1', '::1', '[::1]', '0.0.0.0']);
const PROVIDER_FORMS = ['transcription', 'rewriting', 'meetings'];
// Why speaker labels are off, by the engine's reason code (API.md, SpeakersState).
const SPEAKER_REASONS = {
  off: 'settings.speakers.reason.off',
  platform: 'settings.speakers.reason.platform',
  cpu: 'settings.speakers.reason.cpu',
  runtime: 'settings.speakers.reason.runtime',
  runtimeFailed: 'settings.speakers.reason.runtimeFailed',
  model: 'settings.speakers.reason.model',
  modelDamaged: 'settings.speakers.reason.modelDamaged',
  key: 'settings.speakers.reason.key',
  consent: 'settings.speakers.reason.consent'
};
// How a processor feature reads in Settings. The app names them lower case, as the settings state
// documents; Windows has the x86-64 features, macOS the arm64 ones.
const CPU_FEATURE_NAMES = { avx2: 'AVX2', fma: 'FMA', f16c: 'F16C', bmi2: 'BMI2', neon: 'NEON', dotprod: 'DotProd', fp16: 'FP16' };
const PAGES = ['main', 'transcription', 'dictionary', 'fillers', 'rewriting', 'meetingTranscription', 'speakers', 'storage', 'rukoo', 'import'];
const GROUP_ANCHORS = ['status', 'dictation', 'meetings', 'general', 'about'];

const page = document.getElementById('page');

let state = null;
let stack = ['main'];
let rendered = new Map();
const drafts = {};
const formStatus = {};
const modelLists = {};
let preview = { text: '', for: '' };
let dictionary = { correcting: false, error: '' };
let tokenDone = false;
let importStatus = null;
let idleError = false;
// Bucket sync: the last Test connection on this page (for the fields and secret it ran with), whether the saved
// secret's field is open for a new one, the settings_set and secret calls still running (a test waits for them), the
// connection fields whose value the app refused and which still show it, and the close function of an open key dialog.
let syncTest = null;
let syncTestRun = 0;
let secretVersion = 0;
let syncReplacing = false;
const syncPending = new Set();
const syncRefused = new Set();
let keyDialog = null;

// ---------- small helpers ----------

const current = () => stack[stack.length - 1];

function getPath(object, key) {
  return key.split('.').reduce((value, part) => (value == null ? undefined : value[part]), object);
}

function setPath(object, key, value) {
  const parts = key.split('.');
  const last = parts.pop();
  const target = parts.reduce((value, part) => (value[part] ??= {}), object);
  target[last] = value;
}

const keyName = (key) => (key === 'off' || !key ? t('common.off') : KEY_NAMES[key] ? t(KEY_NAMES[key]) : key.toUpperCase());
const kbd = (key) => `<kbd>${esc(keyName(key))}</kbd>`;
const labelOf = (pairs, value) => {
  const found = pairs.find(([v]) => v === value);
  return found ? t(found[1]) : String(value ?? '');
};

function providerLabel(id, target) {
  if (id === 'custom') return t('settings.provider.custom');
  if (id === 'orukeet') return t(target === 'meetings' ? 'settings.provider.orukeetMeetings' : 'settings.provider.orukeet');
  return BRANDS[id] || id;
}

function hostOf(endpoint) {
  try {
    return new URL(endpoint).hostname.toLowerCase();
  } catch {
    return '';
  }
}

const cut = (text, n = 180) => (text && text.length > n ? `${text.slice(0, n - 1)}…` : text || '');

function status(tone, text) {
  return `<span class="status" data-tone="${tone}"><span class="dot"></span><span>${esc(text)}</span></span>`;
}

// A settings row. With `action` it is a button; otherwise a static row that may hold its own controls in `side`.
function row({ title, desc, descHtml, value, valueClass = '', action, toggle, lead = '', side = '', cls = '', disabled = false, below = '' }) {
  const text = `<div class="text"><div class="title">${esc(title)}</div>${
    descHtml ? `<div class="desc">${descHtml}</div>` : desc ? `<div class="desc">${esc(desc)}</div>` : ''
  }${value ? `<div class="value ${valueClass}">${esc(value)}</div>` : ''}${below}</div>`;
  const sw = toggle === undefined ? '' : `<span class="switch ${toggle ? 'on' : ''}" aria-hidden="true"></span>`;
  if (!action) return `<div class="row static ${cls}">${lead}${text}${side}</div>`;
  const role = toggle === undefined ? '' : `role="switch" aria-checked="${toggle}"`;
  return `<button class="row ${cls}" data-a="${esc(action)}" ${role} ${disabled ? 'disabled' : ''}>${lead}${text}${side}${sw}</button>`;
}

const groupTitle = (title, id = '') => `<div class="settings-group-title" ${id ? `id="group-${id}"` : ''}>${esc(title || '')}</div>`;
const card = (body, cls = '') => `<div class="card ${cls}">${body}</div>`;
const paragraph = (text) => `<p class="settings-text">${esc(text)}</p>`;
const section = (id, html) => ({ id, html });

// ---------- derived values ----------

function dictionaryCount() {
  const { vocabulary, corrections } = state.config.dictionary;
  if (!vocabulary.length && !corrections.length) return '';
  return `${t('settings.dictionary.words', { count: vocabulary.length })} · ${t('settings.dictionary.corrections', { count: corrections.length })}`;
}

function rewritingValue() {
  const { auto } = state.config.rewriting;
  const edits = state.config.dictation.rewriteHotkey !== 'off';
  if (auto && edits) return t('settings.rewriting.valueBoth');
  if (auto) return t('settings.rewriting.valuePolish');
  if (edits) return t('settings.rewriting.valueEdits');
  return t('common.off');
}

function dictationStatus() {
  switch (state.dictation.status) {
    case 'ready':
      return status('ok', t('settings.status.ready'));
    case 'loadingModel':
      return status('busy', t('settings.status.loadingModel'));
    case 'modelMissing':
      return status('bad', t('settings.status.modelMissing'));
    case 'noKey':
      return status('bad', t('settings.status.noKey'));
    default:
      return status('bad', t('settings.status.noMic'));
  }
}

function rukooStatus() {
  switch (state.rukoo.state) {
    case 'connected':
      return status('ok', t('settings.rukoo.connected'));
    case 'rukooOutdated':
      return status('busy', t('settings.rukoo.rukooOutdated'));
    case 'voiceOutdated':
      return status('busy', t('settings.rukoo.voiceOutdated'));
    case 'refused':
      return status('bad', t('settings.rukoo.refused'));
    default:
      return status('off', t('settings.rukoo.notRunning'));
  }
}

function updatesValue() {
  switch (state.updates.state) {
    case 'checking':
      return t('settings.updates.checking');
    case 'available':
      return t('settings.updates.available', { version: state.updates.version });
    case 'upToDate':
      return t('settings.updates.upToDate');
    case 'failed':
      return t('settings.updates.failed');
    default:
      return '';
  }
}

// The keys Voice listens on that the running Ownkey answers too. Voice never stops Ownkey; it only says so.
function ownkeyConflicts() {
  if (!state.ownkey.running) return [];
  const ours = Object.keys(KEY_ROLES).map((role) => getPath(state.config, role));
  return [...new Set(ours.filter((key) => key && key !== 'off' && state.ownkey.keys.includes(key)))];
}

function consentNeeded(provider, endpoint) {
  if (provider === 'orukeet') return false;
  const host = hostOf(endpoint);
  return !host || !LOCAL_HOSTS.has(host);
}

// Whether this is a realtime model (API.md, providers.audio[].streamingPrefix). It streams meetings and does nothing
// else, so the engine refuses it for dictation and rewriting.
function streams(provider, model) {
  const prefix = state.providers.audio.find((p) => p.id === provider)?.streamingPrefix;
  return Boolean(prefix) && String(model || '').trim().startsWith(prefix);
}

// Whether a model's name is a realtime model's, whatever the provider: an OpenAI-compatible proxy can list Mistral's
// ids too, and the engine refuses such a name for dictation and rewriting from any provider (API.md,
// settings_save_provider).
function realtimeName(model) {
  const id = String(model || '').trim();
  return state.providers.audio.some((p) => p.streamingPrefix && id.startsWith(p.streamingPrefix));
}

// How meeting audio goes to the provider: `stream` for a realtime model, else recorded windows. An agreement covers
// one of the two.
function consentMode(provider, model) {
  return streams(provider, model) ? 'stream' : 'windows';
}

function savedConsentFor(provider, endpoint, model) {
  const consent = state.config.meetings.consent;
  return !!consent && consent.provider === provider && consent.host === hostOf(endpoint) && (consent.mode || 'windows') === consentMode(provider, model);
}

function meetingsWaitForConsent() {
  const m = state.config.meetings;
  return consentNeeded(m.provider, m.endpoint) && !savedConsentFor(m.provider, m.endpoint, m.model);
}

// The sentence that says why speaker labels are off, or null when they're ready.
function speakersReason() {
  const s = state.speakers;
  if (!s || s.ready) return null;
  const key = SPEAKER_REASONS[s.reason];
  if (!key) return t('settings.meetings.speakersUnavailable');
  const missing = (s.missing?.length ? s.missing : Object.keys(s.cpu || {}).filter((name) => s.cpu[name] === false)).map((name) => CPU_FEATURE_NAMES[name] || name.toUpperCase());
  return t(key, { missing: list(missing), provider: providerLabel(state.config.meetings.provider, 'meetings') });
}

// The Meetings card's row: On, Unavailable with the reason, or Off.
function speakersRow() {
  const on = state.config.meetings.liveSpeakers;
  const ready = Boolean(state.speakers?.ready);
  return row({
    title: t('settings.meetings.speakers'),
    desc: on && !ready ? speakersReason() : '',
    value: t(!on ? 'common.off' : ready ? 'common.on' : 'settings.meetings.speakersUnavailable'),
    action: 'go:speakers'
  });
}

// ---------- pages ----------

function mainPage() {
  const c = state.config;
  const conflicts = ownkeyConflicts();
  const keysHtml =
    c.dictation.rewriteHotkey === 'off'
      ? tHtml('settings.status.dictationHelpNoRewrite', { key: kbd(c.dictation.hotkey) })
      : tHtml('settings.status.dictationHelp', { key: kbd(c.dictation.hotkey), rewriteKey: kbd(c.dictation.rewriteHotkey) });
  const ready = state.dictation.status === 'ready' || state.dictation.status === 'loadingModel';
  const firstConflictRole = Object.keys(KEY_ROLES).find((role) => conflicts.includes(getPath(c, role)));

  const statusCard = card(
    row({ title: t('settings.status.dictation'), descHtml: keysHtml, side: dictationStatus(), action: ready ? null : 'go:transcription' }) +
      row({ title: t('settings.status.rukoo'), side: rukooStatus(), action: 'go:rukoo' }) +
      (conflicts.length
        ? row({
            lead: `<span class="lead warn top">${icons.warning}</span>`,
            title: t('settings.status.ownkey'),
            desc: t('settings.status.ownkeyHelp', { keys: list(conflicts.map(keyName)) }),
            action: `choose:${firstConflictRole}`
          })
        : '')
  );

  const dictationCard = card(
    row({ title: t('settings.dictation.key'), desc: t('settings.dictation.keyHelp'), value: keyName(c.dictation.hotkey), action: 'choose:dictation.hotkey' }) +
      row({ title: t('settings.dictation.paste'), desc: t('settings.dictation.pasteHelp'), toggle: c.dictation.pasteMode, action: 'toggle:dictation.pasteMode' }) +
      row({ title: t('settings.dictation.chime'), desc: t('settings.dictation.chimeHelp'), toggle: c.dictation.readyChime, action: 'toggle:dictation.readyChime' }) +
      row({
        title: t('settings.dictation.login'),
        desc: state.devMode ? t('settings.dictation.loginDev') : t('settings.dictation.loginHelp'),
        toggle: !state.devMode && c.dictation.startAtLogin,
        action: 'toggle:dictation.startAtLogin',
        disabled: state.devMode
      }) +
      row({ title: t('settings.pill.edge'), value: labelOf(EDGES, c.ui.pillEdge), action: 'choose:ui.pillEdge' }) +
      row({ title: t('settings.pill.hide'), desc: t('settings.pill.hideHelp'), toggle: c.ui.hidePillFromCapture, action: 'toggle:ui.hidePillFromCapture' }) +
      row({ title: t('settings.dictation.transcription'), value: providerLabel(c.transcription.provider, 'transcription'), action: 'go:transcription' }) +
      row({ title: t('settings.dictation.dictionary'), desc: dictionaryCount(), action: 'go:dictionary' }) +
      row({ title: t('settings.dictation.fillers'), value: t(c.fillers.enabled ? 'common.on' : 'common.off'), action: 'go:fillers' }) +
      row({ title: t('settings.dictation.rewriting'), value: rewritingValue(), action: 'go:rewriting' })
  );

  // state.screen is null until the screen process answered its probe. A Windows that can't capture disables the
  // choice; a computer without a hardware encoder or an OCR language records less than the choice says.
  const screen = state.screen;
  const screenOff = screen?.available === false;
  const screenNotes = screenOff
    ? [t('settings.meetings.screenUnavailable')]
    : [screen?.hardwareEncoder === null ? t('settings.meetings.screenNoEncoder') : '', Array.isArray(screen?.ocr) && !screen.ocr.length ? t('settings.meetings.screenNoOcr') : ''].filter(Boolean);
  const screenRow = feature('screen')
    ? row({
        title: t('settings.meetings.screen'),
        desc: t('settings.meetings.screenHelp'),
        below: screenNotes.map((note) => `<div class="desc note">${icons.info}<span>${esc(note)}</span></div>`).join(''),
        side: segmented('meetings.screen', SCREEN, c.meetings.screen, t('settings.meetings.screen'), { disabled: screenOff })
      })
    : '';
  const meetingsCard = card(
    screenRow +
      row({
        title: t('settings.meetings.transcription'),
        desc: meetingsWaitForConsent() ? t('settings.meetings.consentWaiting') : '',
        value: providerLabel(c.meetings.provider, 'meetings'),
        action: 'go:meetingTranscription'
      }) +
      row({ title: t('settings.meetings.retention'), desc: t('settings.meetings.retentionHelp'), value: labelOf(RETENTION, c.meetings.retention), action: 'choose:meetings.retention' }) +
      row({ title: t('settings.meetings.stopKey'), desc: t('settings.meetings.stopKeyHelp'), value: keyName(c.meetings.stopKey), action: 'choose:meetings.stopKey' }) +
      (feature('speakers') ? speakersRow() : '') +
      row({
        title: t('settings.meetings.storage'),
        value: t('settings.meetings.storageValue', { free: bytes(state.storage.freeBytes), size: bytes(state.storage.usedBytes) }),
        action: 'go:storage'
      })
  );

  const generalCard = card(
    row({
      title: t('settings.general.language'),
      value: c.ui.language === 'system' ? t('common.system') : UI_LANGUAGES.find(([code]) => code === c.ui.language)?.[1] || c.ui.language,
      action: 'choose:ui.language'
    }) + row({ title: t('settings.general.theme'), value: labelOf(THEMES, c.ui.theme), action: 'choose:ui.theme' })
  );

  const aboutCard = card(
    row({ title: t('settings.updates.title'), desc: t('settings.updates.help'), toggle: c.updates.check, action: 'toggle:updates.check' }) +
      row({ title: t('settings.updates.checkNow'), value: updatesValue(), action: 'updates-check', disabled: state.updates.state === 'checking' }) +
      row({
        title: t('settings.about.import'),
        value: state.ownkey.lastImport ? t('settings.about.importedOn', { date: dateTime(state.ownkey.lastImport) }) : t('settings.about.notImported'),
        action: 'go:import'
      }) +
      row({ title: t('settings.about.logs'), action: 'open:logs' }) +
      row({ title: t('settings.about.licenses'), action: 'licenses' }) +
      row({ title: t('settings.about.version'), desc: state.devMode ? t('settings.about.dev') : '', side: `<span class="tail selectable">${esc(state.version)}</span>` })
  );

  return {
    title: t('settings.main.title'),
    sections: [
      section('status', groupTitle('', 'status') + statusCard),
      section('dictation', groupTitle(t('settings.groups.dictation'), 'dictation') + dictationCard),
      section('meetings', groupTitle(t('settings.groups.meetings'), 'meetings') + meetingsCard),
      section('general', groupTitle(t('settings.groups.general'), 'general') + generalCard),
      section('about', groupTitle(t('settings.groups.about'), 'about') + aboutCard)
    ]
  };
}

function segmented(key, options, value, label, { disabled = false } = {}) {
  const index = Math.max(0, options.findIndex(([v]) => v === value));
  const off = disabled ? 'disabled' : '';
  return `<div class="segmented ${disabled ? 'is-disabled' : ''}" role="tablist" aria-label="${esc(label)}" ${disabled ? 'aria-disabled="true"' : ''} style="--n:${options.length};--i:${index}">
    <span class="seg-thumb" aria-hidden="true"></span>${options
      .map(([v, labelKey]) => `<button role="tab" aria-selected="${v === value}" data-a="seg:${esc(key)}:${esc(v)}" ${off}>${esc(t(labelKey))}</button>`)
      .join('')}</div>`;
}

function transcriptionPage() {
  const provider = formValue('transcription', 'provider');
  const sections = [
    section('intro', paragraph(t('settings.transcription.intro'))),
    section('form', groupTitle(t('settings.transcription.speechToText')) + card(providerForm('transcription')))
  ];
  if (provider === 'orukeet') sections.push(section('orukeet', groupTitle(t('settings.orukeet.title')) + orukeetCard()));
  return { title: t('settings.dictation.transcription'), sections };
}

// The provider form, for transcription, rewriting and meeting transcription. Ownkey's rules: changing the provider
// clears the key, sets the provider's first endpoint and clears the model, because "Credentials are
// provider-specific. Never carry a key into a newly selected provider where it could be sent accidentally."
function providerForm(target) {
  const saved = state.config[target];
  const d = drafts[target] || {};
  const provider = formValue(target, 'provider');
  const local = provider === 'orukeet';
  const ids = providerIds(target);
  const keySaved = saved.keySaved && provider === saved.provider && !d.replacingKey;
  const presets = presetEndpoints(target, provider);
  const models = modelLists[target];
  // Only the meetings form offers realtime models; the other two would refuse them on Save.
  const offered = (models?.list || []).filter((id) => target === 'meetings' || !realtimeName(id));
  const fs = formStatus[target];

  const options = ids.map((id) => `<option value="${esc(id)}" ${id === provider ? 'selected' : ''}>${esc(providerLabel(id, target))}</option>`).join('');
  let fields = `<label>${esc(t('settings.provider.provider'))}<select data-field="${target}.provider">${options}</select></label>`;

  if (!local) {
    const keyOptional = provider === 'custom' || provider === 'ollama';
    const key = keySaved
      ? `<div class="field"><span>${esc(t('settings.provider.apiKey'))}</span><div class="key-saved"><span>${icons.check}${esc(t('settings.provider.keySaved'))}</span>
          <button class="btn sm secondary" data-a="replace-key:${target}">${esc(t('common.replace'))}</button>
          <button class="btn sm secondary danger" data-a="remove-key:${target}">${esc(t('common.remove'))}</button></div></div>`
      : `<label>${esc(t('settings.provider.apiKey'))}<input type="password" data-field="${target}.apiKey" autocomplete="off" spellcheck="false" placeholder="${esc(
          t('settings.provider.keyPlaceholder')
        )}" />${keyOptional ? `<span class="field-help">${esc(t('settings.provider.keyOptional'))}</span>` : ''}</label>`;
    const modelHelp = !models
      ? ''
      : models.status === 'failed'
        ? `<span class="field-help bad">${esc(t('settings.provider.modelsFailed', { error: models.error }))}</span>`
        : models.status === 'done'
          ? `<span class="field-help">${esc(offered.length ? t('settings.provider.modelsFound', { count: offered.length }) : t('settings.provider.noModels', { provider: providerLabel(provider, target) }))}</span>`
          : '';
    fields += `${key}
      <label>${esc(t('settings.provider.endpoint'))}<input type="text" inputmode="url" list="endpoints-${target}" data-field="${target}.endpoint" spellcheck="false" autocomplete="off" />
        <datalist id="endpoints-${target}">${presets.map((url) => `<option value="${esc(url)}"></option>`).join('')}</datalist></label>
      <label>${esc(t('settings.provider.model'))}<div class="with-btn"><input type="text" list="models-${target}" data-field="${target}.model" spellcheck="false" autocomplete="off" placeholder="${esc(
        t('settings.provider.modelPlaceholder')
      )}" /><button class="btn secondary" data-a="refresh:${target}" ${models?.status === 'loading' ? 'disabled' : ''}>${esc(
        models?.status === 'loading' ? t('common.loading') : t('common.refresh')
      )}</button></div>
        <datalist id="models-${target}">${offered.map((id) => `<option value="${esc(id)}"></option>`).join('')}</datalist>${modelHelp}</label>`;
  }

  if (target === 'transcription') {
    const options = SPOKEN_LANGUAGES.map(
      (code) => `<option value="${code}">${esc(code === 'auto' ? t('settings.transcription.languageAuto') : languageName(code))}</option>`
    ).join('');
    fields += `<label>${esc(t('settings.transcription.language'))}<select data-field="transcription.language" ${local ? 'disabled' : ''}>${options}</select>
      <span class="field-help">${esc(local ? t('settings.transcription.languageOrukeet') : t('settings.transcription.languageHelp'))}</span></label>`;
  }

  if (target === 'meetings') fields += consentBlock(provider, formValue('meetings', 'endpoint'), String(formValue('meetings', 'model')).trim());

  const statusHtml = !fs
    ? ''
    : `<span class="form-status ${fs.kind === 'saved' ? 'ok' : fs.kind === 'failed' || fs.kind === 'invalid' ? 'bad' : ''}">${esc(fs.message)}${
        fs.detail ? ` <button class="link-btn" data-a="details:${target}">${esc(t('common.errorDetails'))}</button>` : ''
      }</span>`;
  const busy = fs && (fs.kind === 'saving' || fs.kind === 'loading');
  fields += `<div class="form-actions">${statusHtml}<button class="btn primary" data-a="save:${target}" ${busy ? 'disabled' : ''}>${esc(t('common.save'))}</button></div>`;
  return `<div class="form" data-form="${target}">${fields}</div>`;
}

// Decision 10: a cloud provider for meetings shows what goes where, with a checkbox. Consent is saved as
// {provider, host, mode}, so another provider, server or way of sending asks again. A realtime model streams the
// meeting, which the disclosure says. Without consent a meeting still records and its transcription waits.
function consentBlock(provider, endpoint, model) {
  if (!consentNeeded(provider, endpoint)) return '';
  const host = hostOf(endpoint);
  const name = providerLabel(provider, 'meetings');
  const stream = consentMode(provider, model) === 'stream';
  const checked = consentValue(provider, endpoint, model);
  return `<div class="consent">
    <h3>${esc(t('settings.consent.title', { provider: name }))}</h3>
    <p>${esc(stream ? t('settings.consent.introStream', { provider: name }) : t('settings.consent.intro', { provider: name }))}</p>
    <dl class="facts">
      <dt>${esc(t('settings.consent.provider'))}</dt><dd>${esc(model ? `${name} · ${model}` : name)}</dd>
      <dt>${esc(t('settings.consent.server'))}</dt><dd class="mono">${esc(host || '–')}</dd>
      <dt>${esc(t('settings.consent.sent'))}</dt><dd>${esc(stream ? t('settings.consent.sentValueStream') : t('settings.consent.sentValue'))}</dd>
      <dt>${esc(t('settings.consent.notSent'))}</dt><dd>${esc(t('settings.consent.notSentValue'))}</dd>
      <dt>${esc(t('settings.consent.kept'))}</dt><dd>${esc(t('settings.consent.keptValue'))}</dd>
    </dl>
    <label class="check-row"><input type="checkbox" data-field="meetings.consent" ${checked ? 'checked' : ''} ${host ? '' : 'disabled'} />
      <span>${esc(
        stream ? t('settings.consent.agreeStream', { provider: name, host: host || '–' }) : t('settings.consent.agree', { provider: name, host: host || '–' })
      )}</span></label>
    <span class="field-help">${esc(checked ? t('settings.consent.again') : t('settings.consent.waiting'))}</span>
  </div>`;
}

// What a ticked box is for: the provider, the host and the way of sending.
const consentKey = (provider, endpoint, model) => `${provider}|${hostOf(endpoint)}|${consentMode(provider, model)}`;

function consentValue(provider, endpoint, model) {
  const d = drafts.meetings;
  if (d && d.consentFor === consentKey(provider, endpoint, model) && typeof d.consent === 'boolean') return d.consent;
  return savedConsentFor(provider, endpoint, model);
}

function providerIds(target) {
  const kind = target === 'rewriting' ? 'rewrite' : 'audio';
  const ids = state.providers[kind].map((p) => p.id);
  // For meetings, Orukeet comes first: it is the default and keeps the audio on this computer.
  return target === 'meetings' ? ['orukeet', ...ids.filter((id) => id !== 'orukeet')] : ids;
}

function presetEndpoints(target, provider) {
  const kind = target === 'rewriting' ? 'rewrite' : 'audio';
  return state.providers[kind].find((p) => p.id === provider)?.endpoints || [];
}

const MODEL_STAGES = { downloading: 'settings.model.downloading', verifying: 'settings.model.verifying', extracting: 'settings.model.extracting' };
const MODEL_ACTIVE = {
  loaded: 'settings.model.state.loaded',
  unloaded: 'settings.model.state.unloaded',
  loading: 'settings.model.state.loading',
  error: 'settings.model.state.error'
};

function modelStatus(m) {
  if (MODEL_STAGES[m.stage]) {
    const stage = t(MODEL_STAGES[m.stage]);
    return { tone: 'busy', text: Number.isFinite(m.percent) ? t('settings.model.progress', { stage, percent: Math.round(m.percent) }) : stage };
  }
  if (m.stage === 'failed') return { tone: 'bad', text: t('settings.model.failed') };
  if (m.stage === 'missing') return { tone: 'off', text: t('settings.model.missing', { size: bytes(m.sizeBytes) }) };
  if (m.stage === 'unavailable') return { tone: 'bad', text: t('settings.model.unavailable') };
  if (m.active === 'error') return { tone: 'bad', text: t('settings.model.loadFailed') };
  if (m.active === 'loading') return { tone: 'busy', text: t('settings.model.loading') };
  if (m.active) return { tone: m.active === 'loaded' ? 'ok' : 'off', text: t('settings.model.active', { state: t(MODEL_ACTIVE[m.active]) }) };
  // Nemotron is never chosen or loaded like Orukeet; the status card above it says whether it labels.
  if (m.id === 'nemotron') return { tone: 'off', text: t('settings.speakers.modelInstalled') };
  return { tone: 'off', text: t('settings.model.installed') };
}

// A model's files, status and actions. "Remove download" only appears for a model in Voice's own folder: Voice
// never deletes anything in Ownkey's folders (architecture.md decision 13).
function modelRows(m) {
  const s = modelStatus(m);
  const busy = Boolean(MODEL_STAGES[m.stage]);
  const progress = busy && Number.isFinite(m.percent) ? `<div class="progress" role="progressbar" aria-valuenow="${Math.round(m.percent)}" aria-valuemin="0" aria-valuemax="100"><span style="--p:${m.percent}"></span></div>` : '';
  // The pill says what state the model is in; the error itself goes on its own line, cut at 180 characters as in
  // Ownkey, with all of it behind Error details.
  const failure = s.tone === 'bad' && m.error ? `<div class="desc">${esc(cut(m.error))}</div>` : '';
  const files = row({
    title: t('settings.model.files'),
    descHtml: m.path ? `<span class="mono selectable">${esc(m.path)}</span>` : '',
    below: progress + failure,
    side: `<span class="model-status">${status(s.tone, s.text)}</span>`
  });
  const actions = [];
  if (busy) actions.push(`<button class="btn sm secondary" data-a="model:${m.id}:cancel">${esc(t('settings.model.cancel'))}</button>`);
  else if (m.stage === 'missing' || m.stage === 'failed') actions.push(`<button class="btn sm primary" data-a="model:${m.id}:download">${icons.download}${esc(t('settings.model.download'))}</button>`);
  if (!busy && m.removable && !m.active && m.stage === 'installed') actions.push(`<button class="btn sm secondary danger" data-a="model:${m.id}:remove">${esc(t('settings.model.remove'))}</button>`);
  if (m.error) actions.push(`<button class="link-btn" data-a="model:${m.id}:details">${esc(t('common.errorDetails'))}</button>`);
  if (m.location === 'ownkey') actions.push(`<span class="model-note">${esc(t('settings.model.inOwnkey'))}</span>`);
  return files + (actions.length ? `<div class="model-actions">${actions.join('')}</div>` : '');
}

function orukeetCard() {
  const m = state.models.orukeet;
  const folder = state.config.local.modelFolder;
  const folderRow = row({
    title: t('settings.model.folder'),
    desc: t('settings.model.folderHelp'),
    below: folder ? `<div class="value plain mono selectable">${esc(folder)}</div>` : `<div class="value plain">${esc(t('settings.model.folderAuto'))}</div>`,
    side: `<div class="side">${folder ? `<button class="btn sm secondary" data-a="folder-reset:models">${esc(t('settings.model.folderReset'))}</button>` : ''}<button class="btn sm secondary" data-a="folder:models">${esc(t('common.browse'))}</button></div>`
  });
  const idleRow = row({
    title: t('settings.model.idle'),
    desc: idleError ? '' : t('settings.model.idleHelp'),
    below: idleError ? `<div class="desc field-error">${esc(t('settings.model.idleInvalid'))}</div>` : '',
    side: `<span class="with-unit"><input class="input" type="number" min="0" max="1440" step="1" data-field="local.idleMinutes" aria-label="${esc(
      t('settings.model.idle')
    )}" /><span>${esc(t('settings.model.minutes'))}</span></span>`
  });
  const foot = `<div class="card-foot">${esc(t('settings.orukeet.licence'))} <button class="link-btn" data-a="link:orukeet">${icons.link}${esc(t('settings.model.page'))}</button></div>`;
  return card(`<p class="settings-text card-text">${esc(t('settings.orukeet.about'))}</p>${modelRows(m)}${folderRow}${idleRow}${foot}`, 'model-card');
}

function dictionaryPage() {
  const { vocabulary, corrections } = state.config.dictionary;
  const correcting = dictionary.correcting;
  const entries = [
    ...vocabulary.map((word) => ({ kind: 'word', sort: word.toLowerCase(), word })),
    ...corrections.map((c) => ({ kind: 'fix', sort: c.to.toLowerCase(), from: c.from, to: c.to }))
  ].sort((a, b) => a.sort.localeCompare(b.sort, language()));
  const fields = correcting
    ? `<label>${esc(t('settings.dictionary.from'))}<input type="text" data-field="dict.from" spellcheck="false" autocomplete="off" /></label>
       <span class="dict-arrow" aria-hidden="true">→</span>
       <label>${esc(t('settings.dictionary.to'))}<input type="text" data-field="dict.to" spellcheck="false" autocomplete="off" /></label>`
    : `<label>${esc(t('settings.dictionary.word'))}<input type="text" data-field="dict.word" spellcheck="false" autocomplete="off" placeholder="${esc(
        t('settings.dictionary.wordPlaceholder')
      )}" /></label>`;
  const add = `<div class="form"><div class="dict-form">${fields}<button class="btn primary" data-a="dict-add">${esc(
    t(correcting ? 'settings.dictionary.addCorrection' : 'settings.dictionary.addWord')
  )}</button></div>${dictionary.error ? `<span class="field-help bad" role="alert">${esc(dictionary.error)}</span>` : ''}</div>`;
  const rows = entries.length
    ? entries
        .map((e) => {
          const label = e.kind === 'word' ? e.word : e.from;
          const title =
            e.kind === 'word'
              ? esc(e.word)
              : `<span>${esc(e.from)}</span><span class="arrow" aria-hidden="true">→</span><span class="to">${esc(e.to)}</span>`;
          return `<div class="row static dict-row"><div class="text"><div class="title">${title}</div></div>
            <span class="tag">${esc(t(e.kind === 'word' ? 'settings.dictionary.hint' : 'settings.dictionary.fix'))}</span>
            <button class="icon-btn sm" data-a="dict-remove:${e.kind}:${esc(encodeURIComponent(label))}" title="${esc(
              t('settings.dictionary.removeEntry', { entry: label })
            )}" aria-label="${esc(t('settings.dictionary.removeEntry', { entry: label }))}">${icons.trash}</button></div>`;
        })
        .join('')
    : `<p class="settings-text card-text">${esc(t('settings.dictionary.empty'))}</p>`;
  return {
    title: t('settings.dictation.dictionary'),
    sections: [
      section('intro', paragraph(t('settings.dictionary.intro'))),
      section(
        'list',
        groupTitle(dictionaryCount() || t('settings.dictionary.vocabulary')) +
          card(row({ title: t('settings.dictionary.correct'), desc: t('settings.dictionary.correctHelp'), toggle: correcting, action: 'dict-mode' }) + add + rows)
      ),
      section('how', groupTitle(t('settings.dictionary.how')) + paragraph(t('settings.dictionary.howWords')) + paragraph(t('settings.dictionary.howCorrections')))
    ]
  };
}

function fillersPage() {
  const f = state.config.fillers;
  const languages = Object.keys(FILLER_WORDS)
    .map((code) =>
      row({
        title: languageName(code),
        desc: FILLER_WORDS[code].join(', '),
        toggle: f.languages.includes(code),
        action: `filler-language:${code}`,
        disabled: !f.enabled
      })
    )
    .join('');
  return {
    title: t('settings.dictation.fillers'),
    sections: [
      section('intro', paragraph(t('settings.fillers.intro'))),
      section('main', groupTitle('') + card(row({ title: t('settings.fillers.remove'), desc: t('settings.fillers.removeHelp'), toggle: f.enabled, action: 'toggle:fillers.enabled' }))),
      section('languages', groupTitle(t('settings.fillers.languages')) + card(languages)),
      section(
        'custom',
        groupTitle('') +
          card(`<div class="form"><label>${esc(t('settings.fillers.custom'))}<input type="text" data-field="fillers.custom" spellcheck="false" autocomplete="off" placeholder="${esc(
            t('settings.fillers.customPlaceholder')
          )}" /><span class="field-help">${esc(t('settings.fillers.customHelp'))}</span></label></div>`)
      ),
      section(
        'preview',
        groupTitle(t('settings.fillers.preview')) +
          card(
            row({ title: t('settings.fillers.heard'), below: `<div class="preview-line heard">${esc(FILLER_SAMPLE)}</div>` }) +
              row({ title: t('settings.fillers.typed'), below: `<div class="preview-line typed">${esc(preview.text || FILLER_SAMPLE)}</div>` })
          )
      )
    ]
  };
}

function rewritingPage() {
  const r = state.config.rewriting;
  return {
    title: t('settings.dictation.rewriting'),
    sections: [
      section('intro', paragraph(t('settings.rewriting.intro'))),
      section('form', groupTitle(t('settings.rewriting.textModel')) + `<p class="settings-text">${esc(t('settings.rewriting.textModelHelp'))}</p>` + card(providerForm('rewriting'))),
      section(
        'polish',
        groupTitle(t('settings.rewriting.polish')) +
          card(
            row({ title: t('settings.rewriting.auto'), desc: t('settings.rewriting.autoHelp'), toggle: r.auto, action: 'toggle:rewriting.auto' }) +
              row({ title: t('settings.rewriting.tone'), value: labelOf(TONES, r.tone), action: 'choose:rewriting.tone' }) +
              row({ title: t('settings.rewriting.formatting'), desc: t('settings.rewriting.formattingHelp'), toggle: r.formatting, action: 'toggle:rewriting.formatting' }) +
              `<div class="form"><label>${esc(t('settings.rewriting.instructions'))}<input type="text" data-field="rewriting.instructions" autocomplete="off" placeholder="${esc(
                t('settings.rewriting.instructionsPlaceholder')
              )}" /></label></div>`
          )
      ),
      section(
        'edits',
        groupTitle(t('settings.rewriting.voiceEdits')) +
          card(row({ title: t('settings.rewriting.key'), desc: t('settings.rewriting.keyHelp'), value: keyName(state.config.dictation.rewriteHotkey), action: 'choose:dictation.rewriteHotkey' }))
      )
    ]
  };
}

function meetingTranscriptionPage() {
  return {
    title: t('settings.meetingTranscription.title'),
    sections: [section('intro', paragraph(t('settings.meetingTranscription.intro'))), section('form', groupTitle('') + card(providerForm('meetings')))]
  };
}

// The reasons whose error text Error details shows.
const SPEAKER_DETAILS = new Set(['runtimeFailed', 'modelDamaged']);

// Whether labels are on and why not, where the labels come from, then the Nemotron model with the switch. The switch
// stays usable without the model: it decides what new meetings do once labels can run.
function speakersPage() {
  const m = state.models.nemotron;
  const s = state.speakers || {};
  const on = state.config.meetings.liveSpeakers;
  const mistral = s.engine === 'mistral';
  const ready = Boolean(s.ready);
  const pill = ready
    ? status('ok', t('common.on'))
    : on
      ? status('bad', t('settings.meetings.speakersUnavailable'))
      : status('off', t('common.off'));
  const details =
    !ready && s.detail && SPEAKER_DETAILS.has(s.reason)
      ? `<div class="desc"><button class="link-btn" data-a="speakers-details">${esc(t('common.errorDetails'))}</button></div>`
      : '';
  const stopped = (s.live || []).some((entry) => entry.state === 'failed');
  const statusCard = card(
    row({
      title: t('settings.speakers.status'),
      desc: ready ? t(mistral ? 'settings.speakers.readyMistral' : 'settings.speakers.ready') : speakersReason(),
      below: details,
      side: pill
    }) +
      row({
        title: t('settings.speakers.engine'),
        value: t(mistral ? 'settings.speakers.engineMistral' : 'settings.speakers.engineLocal'),
        valueClass: mistral ? '' : 'plain',
        action: mistral ? 'go:meetingTranscription' : null
      }) +
      (stopped ? row({ lead: `<span class="lead warn top">${icons.warning}</span>`, title: t('settings.speakers.stopped') }) : '')
  );
  return {
    title: t('settings.meetings.speakers'),
    sections: [
      section('status', groupTitle('') + statusCard),
      section(
        'model',
        groupTitle('Nemotron 3 Diarization') +
          card(
            `<p class="settings-text card-text">${esc(t('settings.speakers.about'))}</p>${modelRows(m)}` +
              row({ title: t('settings.speakers.live'), desc: t('settings.speakers.liveHelp'), toggle: on, action: 'toggle:meetings.liveSpeakers' }) +
              `<div class="card-foot">${esc(t('settings.speakers.licence'))} <button class="link-btn" data-a="link:nemotron">${icons.link}${esc(t('settings.model.page'))}</button></div>`
          )
      )
    ]
  };
}

function storagePage() {
  const s = state.storage;
  const folderRow = row({
    title: t('settings.storage.folder'),
    descHtml: `<span class="mono selectable">${esc(s.folder)}</span>`,
    below: `<div class="desc">${esc(t('settings.storage.usage', { size: bytes(s.usedBytes), free: bytes(s.freeBytes) }))}</div>`,
    // Voice 0.1 keeps meetings in its data folder, so there is no Browse here (API.md, settings_choose_folder).
    side: `<div class="side"><button class="btn sm secondary" data-a="open:recordings">${esc(t('common.openFolder'))}</button></div>`
  });
  const sections = [section('local', groupTitle(t('settings.storage.local')) + card(folderRow) + (feature('screen') ? paragraph(t('settings.storage.disk')) : ''))];
  if (feature('sync')) sections.push(...syncSections());
  else if (state.config.storage.sync?.enabled) sections.push(section('sync', groupTitle(t('settings.sync.inBucket')) + syncUnavailableCard()));
  return { title: t('settings.meetings.storage'), sections };
}

// The sync store didn't open (locked, damaged, or from a newer Voice) while the switch is on: nothing uploads and
// retention keeps every meeting's media. The switch stays here, because turning it off is the way to let that media go
// (API.md, sync.available).
function syncUnavailableCard() {
  const error = state.sync?.lastError;
  const details = error?.message ? ` <button class="link-btn" data-a="sync-details:unavailable">${esc(t('common.errorDetails'))}</button>` : '';
  return card(
    row({ title: t('settings.sync.title'), toggle: true, action: 'toggle:storage.sync.enabled', cls: 'sync-switch' }) +
      `<div class="sync-status">${status('bad', t('settings.sync.unavailable'))}<p class="sync-note">${esc(t('settings.sync.unavailableHeld'))}${details}</p></div>`
  );
}

// ---------- bucket sync ----------

// Settings > Storage, in your bucket: the switch and what the bucket is doing, the connection, what leaves this
// computer, the encryption key, and how long the bucket keeps media. Five sections, so a new waiting count redraws
// only the first card and never the form someone is typing in. The engine refuses to turn sync on until the current
// fields and secret passed Test connection. When one of them changes while sync is on, uploads pause until a test
// passes again, and the switch and retention's hold stay as they were (API.md).
function syncSections() {
  const check = `<span class="lead">${icons.check}</span>`;
  return [
    section('sync', groupTitle(t('settings.sync.inBucket')) + syncSwitchCard()),
    section('sync-connection', groupTitle('') + syncConnectionCard()),
    section('sync-privacy', groupTitle('') + card(row({ title: t('settings.sync.encrypted'), lead: check }) + row({ title: t('settings.sync.videoAfter'), lead: check }))),
    section('sync-key', groupTitle('') + syncKeyCard()),
    section('sync-bucket', groupTitle('') + syncBucketCard())
  ];
}

// The switch turns on only for the fields on screen. While one of them shows a value the app refused, sync would start
// with the saved value instead, so it waits; turning sync off never does.
function syncSwitchCard() {
  const sync = state.config.storage.sync;
  const refused = !sync.enabled && syncRefused.size > 0;
  const waits = (!sync.tested && !sync.enabled) || refused;
  const desc = refused
    ? t('settings.sync.refused')
    : waits
      ? t('settings.sync.needsTest')
      : sync.enabled
        ? t('settings.sync.help')
        : `${t('settings.sync.help')} ${t('settings.sync.backfill')}`;
  const lines = syncStatus();
  return card(
    row({ title: t('settings.sync.title'), desc, toggle: sync.enabled, action: 'toggle:storage.sync.enabled', disabled: waits, cls: 'sync-switch' }) +
      (lines ? `<div class="sync-status">${lines}</div>` : '')
  );
}

// What the bucket is doing, while sync is on, and what never reached it in any state. While uploads can't go, the count
// of files still waiting shows too: those are what the 10 GB floor could cost.
function syncStatus() {
  const s = state.sync;
  if (!s) return '';
  const note = (html, cls = '') => `<p class="sync-note ${cls}">${html}</p>`;
  const details = (which) => ` <button class="link-btn" data-a="sync-details:${which}">${esc(t('common.errorDetails'))}</button>`;
  const waiting = () => (s.waiting > 0 ? note(esc(t('settings.sync.waiting', { count: s.waiting })), 'quiet') : '');
  const parts = [];
  if (state.config.storage.sync.enabled) {
    switch (s.state) {
      case 'idle':
        parts.push(status('ok', t('settings.sync.upToDate')));
        if (s.lastUpload) parts.push(note(esc(t('settings.sync.lastUpload', { time: dateTime(s.lastUpload) })), 'quiet'));
        break;
      case 'uploading':
        parts.push(status('busy', t('settings.sync.waiting', { count: s.waiting })));
        break;
      case 'live':
        parts.push(status('busy', t('settings.sync.live')), note(esc(t('settings.sync.waiting', { count: s.waiting })), 'quiet'));
        break;
      case 'retrying':
        parts.push(status('busy', t('settings.sync.retrying')), waiting());
        if (s.oldestWaiting) parts.push(note(esc(t('settings.sync.waitingSince', { time: dateTime(s.oldestWaiting) })) + (s.lastError?.message ? details('last') : ''), 'quiet'));
        parts.push(note(esc(t('settings.sync.keptLocal'))));
        break;
      case 'failing': {
        // "The test failed." would say the wrong thing about an upload, so `failed` gets Error details only.
        const reason = s.lastError?.code !== 'failed' && SYNC_ERRORS[s.lastError?.code];
        parts.push(status('bad', t('settings.sync.stopped')));
        if (reason || s.lastError?.message) parts.push(note((reason ? esc(t(reason)) : '') + (s.lastError?.message ? details('last') : '')));
        parts.push(waiting(), note(esc(t('settings.sync.keptLocal'))));
        break;
      }
      case 'key':
        parts.push(status('bad', t('settings.sync.keyUnreadable')), `<button class="btn sm secondary" data-a="sync-key-use">${esc(t('settings.sync.keyUse'))}</button>`);
        break;
      case 'paused':
        parts.push(status('bad', t('settings.sync.paused')), waiting(), note(esc(t('settings.sync.keptLocal'))));
        break;
    }
  }
  if (s.lost > 0) parts.push(note(`${icons.warning}<span>${esc(t('settings.sync.lostLocal', { count: s.lost }))}</span>`, 'warn'));
  return parts.join('');
}

// The fields save on blur through settings_set. The secret never does: it goes to settings_sync_save_secret and
// then shows as Key saved, like a provider's API key. A field whose value the app refused is marked, and Test connection
// waits until a value saves there: the test would run with the saved value, not the one on screen.
function syncConnectionCard() {
  const sync = state.config.storage.sync;
  const service = formValue('storage', 'sync.service') || 'upcloud';
  const field = (name, label, placeholder = '', extra = '') =>
    `<label>${esc(t(label))}<input type="text" data-field="storage.sync.${name}" spellcheck="false" autocomplete="off" ${extra} ${
      placeholder ? `placeholder="${esc(placeholder)}"` : ''
    } ${syncRefused.has(`sync.${name}`) ? 'aria-invalid="true"' : ''} /></label>`;
  const services = [
    ['upcloud', 'UpCloud Managed Object Storage'],
    ['r2', 'Cloudflare R2'],
    ['other', t('settings.sync.other')]
  ];
  const secret =
    sync.secretSaved && !syncReplacing
      ? `<div class="field"><span>${esc(t('settings.sync.secretKey'))}</span><div class="key-saved"><span>${icons.check}${esc(t('settings.provider.keySaved'))}</span>
          <button class="btn sm secondary" data-a="sync-secret-replace">${esc(t('common.replace'))}</button>
          <button class="btn sm secondary danger" data-a="sync-secret-remove">${esc(t('common.remove'))}</button></div></div>`
      : `<label>${esc(t('settings.sync.secretKey'))}<input type="password" data-field="storage.sync.secret" autocomplete="off" spellcheck="false" /></label>`;
  const test = currentSyncTest();
  const statusHtml =
    test?.kind === 'testing'
      ? `<span class="form-status grow">${esc(t('settings.sync.testing'))}</span>`
      : test?.kind === 'ok'
        ? `<span class="form-status grow ok">${icons.check}<span>${esc(t('settings.sync.testOk'))}</span></span>`
        : test?.kind === 'failed'
          ? `<span class="form-status grow bad"><span>${esc(t(SYNC_ERRORS[test.code] || 'settings.sync.error.failed'))}${
              test.message ? ` <button class="link-btn" data-a="sync-details:test">${esc(t('common.errorDetails'))}</button>` : ''
            }</span></span>`
          : syncRefused.size
            ? `<span class="form-status grow bad"><span>${esc(t('settings.sync.refused'))}</span></span>`
            : '<span class="form-status grow"></span>';
  const otherKeys = test?.kind === 'ok' && test.otherKeys > 0 ? `<span class="field-help">${esc(t('settings.sync.otherKeys'))}</span>` : '';
  return card(`<div class="form" data-form="sync">
    <label>${esc(t('settings.sync.service'))}<select data-field="storage.sync.service">${services.map(([v, l]) => `<option value="${v}">${esc(l)}</option>`).join('')}</select></label>
    ${field('endpoint', 'settings.sync.endpoint', SYNC_ENDPOINTS[service], 'inputmode="url"')}${field('region', 'settings.sync.region', service === 'r2' ? 'auto' : '')}
    ${field('bucket', 'settings.sync.bucket')}${field('prefix', 'settings.sync.prefix')}${field('accessKeyId', 'settings.sync.accessKey')}
    ${secret}${otherKeys}
    <div class="form-actions">${statusHtml}<button class="btn primary" data-a="sync-test" ${test?.kind === 'testing' || !sync.secretSaved || syncRefused.size ? 'disabled' : ''}>${esc(
      t('settings.sync.test')
    )}</button></div></div>`);
}

// A test result counts only for the fields and the secret it ran with. Those are the saved fields, so none shows while
// a field holds a value the app refused.
const syncConnectionKey = () => JSON.stringify([...SYNC_FIELDS.map((name) => state.config.storage.sync[name] ?? ''), secretVersion]);
const currentSyncTest = () => (syncTest && !syncRefused.size && syncTest.for === syncConnectionKey() ? syncTest : null);

function syncKeyCard() {
  const s = state.sync || {};
  const key = s.key || {};
  const saved = key.exported
    ? `<div class="desc">${esc(t('settings.sync.keySaved', { id: key.id || '–' }))}</div>`
    : `<div class="desc key-note">${esc(t('settings.sync.keyNotSaved'))}</div>`;
  // A bucket with meetings under another key: on a new computer, that is the key to bring over.
  const other = s.otherKeys > 0 ? `<div class="desc">${esc(t('settings.sync.otherKeys'))}</div>` : '';
  return card(
    row({ title: t('settings.sync.recovery'), desc: t('settings.sync.recoveryHelp'), below: saved + other }) +
      `<div class="card-actions"><button class="btn sm ${key.exported ? 'secondary' : 'primary'}" data-a="sync-key-show" ${s.state === 'key' ? 'disabled' : ''}>${esc(
        t('settings.sync.keyShow')
      )}</button><button class="btn sm secondary" data-a="sync-key-use">${esc(t('settings.sync.keyUse'))}</button></div>`
  );
}

function syncBucketCard() {
  const sync = state.config.storage.sync;
  const retention = row({
    title: t('settings.sync.retention'),
    desc: t('settings.sync.retentionHelp'),
    side: segmented('storage.sync.retention', SYNC_RETENTION, sync.retention || 'keep', t('settings.sync.retention'))
  });
  return card(retention + restoreRow(state.sync?.restore || {}));
}

// Meetings in the bucket that aren't on this computer, and the restore once it started.
function restoreRow(r) {
  const details = r.error ? ` <button class="link-btn" data-a="sync-details:restore">${esc(t('common.errorDetails'))}</button>` : '';
  if (r.state === 'restoring') {
    const percent = r.total ? (100 * r.done) / r.total : 0;
    return row({
      title: t('settings.sync.restoring', { done: r.done ?? 0, total: r.total ?? 0 }),
      below: `<div class="progress" role="progressbar" aria-valuenow="${Math.round(percent)}" aria-valuemin="0" aria-valuemax="100"><span style="--p:${percent}"></span></div>`
    });
  }
  const failed = r.state === 'failed' ? `<div class="desc field-error">${esc(t('settings.sync.restoreFailed'))}${details}</div>` : '';
  if (r.available > 0) {
    return row({
      title: t('settings.sync.restoreAvailable', { count: r.available }),
      desc: t('settings.sync.restoreHelp'),
      below: failed,
      side: `<button class="btn sm primary" data-a="sync-restore">${esc(t('settings.sync.restore'))}</button>`
    });
  }
  if (r.state === 'failed') return row({ lead: `<span class="lead warn">${icons.warning}</span>`, title: t('settings.sync.restoreFailed'), descHtml: details.trim() });
  if (r.state === 'done') return row({ lead: `<span class="lead ok">${icons.check}</span>`, title: t('settings.sync.restored') });
  return '';
}

function rukooPage() {
  const r = state.rukoo;
  const help = {
    connected: r.clientVersion ? t('settings.rukoo.version', { version: r.clientVersion, protocol: r.protocol }) : '',
    notRunning: t('settings.rukoo.notRunningHelp'),
    rukooOutdated: t('settings.rukoo.rukooOutdatedHelp'),
    voiceOutdated: t('settings.rukoo.voiceOutdatedHelp'),
    refused: t('settings.rukoo.refusedHelp')
  }[r.state];
  const connection = card(
    row({ title: t('settings.status.rukoo'), desc: help, side: rukooStatus() }) +
      row({ title: t('settings.rukoo.address'), side: `<span class="tail mono selectable">${esc(r.port ? `127.0.0.1:${r.port}` : '–')}</span>` }) +
      row({
        title: t('settings.rukoo.file'),
        descHtml: `<span class="mono selectable">${esc(r.discoveryPath)}</span>`,
        side: `<button class="btn sm secondary" data-a="open:bridge">${esc(t('common.openFolder'))}</button>`
      }) +
      (r.state === 'connected' ? row({ title: t('settings.rukoo.behind'), side: `<span class="tail">${esc(String(r.behind ?? 0))}</span>` }) : '')
  );
  const token = card(
    row({
      title: t('settings.rukoo.token'),
      desc: tokenDone ? t('settings.rukoo.newTokenDone') : t('settings.rukoo.newTokenHelp'),
      side: `<button class="btn sm secondary" data-a="new-token">${esc(t('settings.rukoo.newToken'))}</button>`
    })
  );
  return {
    title: t('settings.status.rukoo'),
    sections: [
      section('connection', groupTitle(t('settings.rukoo.connection')) + connection + paragraph(t('settings.rukoo.about'))),
      section('token', groupTitle('') + token)
    ]
  };
}

function importPage() {
  const o = state.ownkey;
  const report = o.report;
  const modelList = o.models.length ? list(o.models.map((m) => `${m.name} (${bytes(m.sizeBytes)})`)) : t('settings.import.noModels');
  const found = card(
    row({
      title: t('settings.import.config'),
      descHtml: o.configFound ? tHtml('settings.import.found', { path: `<span class="mono selectable">${esc(o.configPath)}</span>` }) : esc(t('settings.import.notFound'))
    }) +
      row({ title: t('settings.import.last'), side: `<span class="tail">${esc(o.lastImport ? dateTime(o.lastImport) : t('settings.import.never'))}</span>` }) +
      row({ title: t('settings.import.models'), desc: `${modelList}. ${o.models.length ? t('settings.import.modelsHelp') : ''}`.trim() })
  );
  const sections = [section('found', groupTitle('') + found)];
  if (report) {
    const area = (title, value, extra) =>
      row({ title, descHtml: value ? esc(value) + (extra ? `<br>${esc(extra)}` : '') : esc(t('settings.import.notSet')) });
    const providerLine = (p, target) => (p ? [providerLabel(p.provider, target), p.model].filter(Boolean).join(' · ') : '');
    const keyLine = (p) => (p?.keyImported ? t('settings.import.keyImported', { provider: providerLabel(p.provider) }) : '');
    const rows =
      area(t('settings.import.keys'), report.hotkeys && t('settings.import.keysValue', { key: keyName(report.hotkeys.dictation), rewriteKey: keyName(report.hotkeys.rewrite) })) +
      area(t('settings.dictation.transcription'), providerLine(report.transcription, 'transcription'), keyLine(report.transcription)) +
      area(t('settings.dictation.rewriting'), providerLine(report.rewriting, 'rewriting'), keyLine(report.rewriting)) +
      area(
        t('settings.dictation.dictionary'),
        report.dictionary &&
          `${t('settings.dictionary.words', { count: report.dictionary.words })} · ${t('settings.dictionary.corrections', { count: report.dictionary.corrections })}`
      ) +
      area(
        t('settings.dictation.fillers'),
        report.fillers && (report.fillers.enabled ? t('settings.import.fillersOn', { languages: list(report.fillers.languages.map(languageName)) }) : t('common.off'))
      ) +
      area(
        t('settings.groups.meetings'),
        report.meetings && `${providerLabel(report.meetings.provider, 'meetings')} · ${t('settings.import.retention', { retention: labelOf(RETENTION, report.meetings.retention) })}`,
        report.meetings && report.meetings.provider !== 'orukeet' ? t(report.meetings.consent ? 'settings.import.consentKept' : 'settings.import.consentAsk') : ''
      );
    sections.push(section('report', groupTitle(t('settings.import.tookOver')) + card(rows)));
    if (report.skipped?.includes('summaries')) {
      sections.push(
        section('skipped', groupTitle(t('settings.about.notImported')) + card(row({ title: t('settings.import.summaries'), desc: t('settings.import.summariesWhy') })))
      );
    }
  }
  const failed = importStatus?.kind === 'failed' ? `<span class="field-error">${esc(t('settings.import.failed', { error: importStatus.message }))}</span>` : '';
  const action = `<div class="page-actions"><p>${esc(state.devMode ? t('settings.import.dev') : t('settings.import.readOnly'))}${failed ? `<br>${failed}` : ''}</p>
    <button class="btn primary" data-a="import" ${state.devMode || !o.configFound || importStatus?.kind === 'running' ? 'disabled' : ''}>${esc(
      t(o.lastImport ? 'settings.import.again' : 'settings.import.run')
    )}</button></div>`;
  sections.push(section('action', action));
  return { title: t('settings.about.import'), sections };
}

const PAGE_BUILDERS = {
  main: mainPage,
  transcription: transcriptionPage,
  dictionary: dictionaryPage,
  fillers: fillersPage,
  rewriting: rewritingPage,
  meetingTranscription: meetingTranscriptionPage,
  speakers: speakersPage,
  storage: storagePage,
  rukoo: rukooPage,
  import: importPage
};

// ---------- rendering ----------

// A redraw between a button's mousedown and its click puts a new button in its place, and Chromium drops the click.
// It happens when a field saves because the press took the focus from it: the bucket name fixed after a failed Test
// connection, then Test clicked, would only clear the old result. While a button is pressed, redraws wait until its
// click has gone out.
let pressing = false;
let redrawAfterPress = null;
document.addEventListener('pointerdown', (e) => (pressing = Boolean(e.target.closest?.('button, [data-a]'))), true);
function pressDone() {
  if (!pressing) return;
  pressing = false;
  if (!redrawAfterPress) return;
  const wanted = redrawAfterPress;
  redrawAfterPress = null;
  // A task later, so after the click this press makes.
  setTimeout(() => render(wanted));
}
document.addEventListener('pointerup', pressDone, true);
document.addEventListener('pointercancel', pressDone, true);
window.addEventListener('blur', pressDone);

// Builds the current page. A full render replaces everything; an update replaces only the sections whose markup
// changed, so a download's progress doesn't reset the form next to it. Typed text isn't part of the markup: it
// lives in drafts and fill() puts it back, so it survives any re-render.
function render({ full = false } = {}) {
  if (!state) return;
  if (pressing) {
    redrawAfterPress = { full: full || Boolean(redrawAfterPress?.full) };
    return;
  }
  const view = PAGE_BUILDERS[current()]();
  const top = current() === 'main';
  if (full || !page.querySelector('.page-inner') || page.dataset.view !== current()) {
    page.dataset.view = current();
    page.innerHTML = `<div class="page-bar ${top ? 'top' : ''}">${
      top ? '' : `<button class="icon-btn" data-a="back" title="${esc(t('common.back'))}" aria-label="${esc(t('common.back'))}">${icons.back}</button>`
    }<h1>${esc(view.title)}</h1></div><div class="page-scroll"><div class="page-inner">${view.sections
      .map((s) => `<section data-section="${s.id}">${s.html}</section>`)
      .join('')}</div></div>`;
    rendered = new Map(view.sections.map((s) => [s.id, s.html]));
    fill(page);
    return;
  }
  page.querySelector('.page-bar h1').textContent = view.title;
  const inner = page.querySelector('.page-inner');
  const ids = view.sections.map((s) => s.id);
  // A section that appears or disappears, such as the Orukeet card, rebuilds the list in order.
  if (ids.join() !== [...rendered.keys()].join()) {
    const keep = captureFocus();
    inner.innerHTML = view.sections.map((s) => `<section data-section="${s.id}">${s.html}</section>`).join('');
    rendered = new Map(view.sections.map((s) => [s.id, s.html]));
    fill(inner);
    restoreFocus(keep);
    return;
  }
  for (const s of view.sections) {
    if (rendered.get(s.id) === s.html) continue;
    const node = inner.querySelector(`[data-section="${s.id}"]`);
    const keep = node.contains(document.activeElement) ? captureFocus() : null;
    node.innerHTML = s.html;
    rendered.set(s.id, s.html);
    fill(node);
    if (keep) restoreFocus(keep);
  }
}

function captureFocus() {
  const el = document.activeElement;
  if (!el || el === document.body) return null;
  const selector = el.dataset.field ? `[data-field="${el.dataset.field}"]` : el.dataset.a ? `[data-a="${el.dataset.a}"]` : null;
  if (!selector) return null;
  let selection = null;
  try {
    selection = el.selectionStart != null ? [el.selectionStart, el.selectionEnd] : null;
  } catch {
    selection = null;
  }
  return { selector, selection };
}

function restoreFocus(keep) {
  if (!keep) return;
  const el = page.querySelector(keep.selector);
  if (!el) return;
  el.focus({ preventScroll: true });
  if (keep.selection) {
    try {
      el.setSelectionRange(...keep.selection);
    } catch {
      // number inputs have no selection
    }
  }
}

// The value a form field shows: the user's draft while they edit, otherwise the app's value.
function formValue(form, name) {
  const draft = drafts[form]?.[name];
  if (draft !== undefined) return draft;
  if (PROVIDER_FORMS.includes(form)) {
    if (name === 'apiKey') return '';
    return state.config[form][name] ?? '';
  }
  if (form === 'dict') return '';
  return getPath(state.config, `${form}.${name}`) ?? '';
}

function fill(root) {
  for (const el of root.querySelectorAll('[data-field]')) {
    const [form, ...rest] = el.dataset.field.split('.');
    const name = rest.join('.');
    if (el.type === 'checkbox') continue;
    const value = formValue(form, name);
    if (el.value !== String(value)) el.value = String(value);
  }
}

// ---------- changing values ----------

// Saves one value, showing it at once and rolling it back when the app refuses it. Resolves to whether it saved.
async function setValue(key, value) {
  const before = getPath(state.config, key);
  setPath(state.config, key, value);
  render();
  try {
    await invoke('settings_set', { key, value });
    return true;
  } catch (error) {
    setPath(state.config, key, before);
    render();
    const message =
      error?.code === 'same_key'
        ? t('settings.rewriting.sameKey')
        : error?.code === 'needs_test'
          ? t('settings.sync.needsTest')
          : error?.code === 'invalid_value' && key === 'storage.sync.endpoint'
            ? t('settings.sync.error.endpoint')
            : t('settings.errors.failed');
    await showError(message, error?.message);
    return false;
  }
}

async function choose({ title, options, current: value, note }) {
  return dialog({
    title,
    body: `${note ? `<p class="note">${esc(note)}</p>` : ''}<div class="choices" role="radiogroup">${options
      .map(
        (o, i) =>
          `<button class="radio-row" role="radio" aria-checked="${o.value === value}" data-i="${i}" ${o.disabled ? 'disabled' : ''}><span class="radio ${
            o.value === value ? 'on' : ''
          }"></span><span class="rt"><span class="rl">${esc(o.label)}</span>${o.hint ? `<span class="rh">${esc(o.hint)}</span>` : ''}</span></button>`
      )
      .join('')}</div>`,
    buttons: [{ label: t('common.cancel'), value: null }],
    onRender(body, done) {
      body.querySelectorAll('.radio-row').forEach((b) => b.addEventListener('click', () => done(options[Number(b.dataset.i)].value)));
      (body.querySelector('.radio.on')?.parentElement || body.querySelector('.radio-row:not(:disabled)'))?.focus();
    }
  });
}

// A key list greys out the keys another Voice key already uses, instead of Ownkey's silent switch to Off, and says
// which keys the running Ownkey answers too.
async function keyChoice(role) {
  const c = state.config;
  const taken = Object.fromEntries(
    Object.keys(KEY_ROLES)
      .filter((other) => other !== role)
      .map((other) => [getPath(c, other), KEY_ROLES[other]])
      .filter(([key]) => key && key !== 'off')
  );
  const ownkeyKeys = state.ownkey.running ? state.ownkey.keys : [];
  const keys = role === 'dictation.hotkey' ? HOTKEYS : ['off', ...HOTKEYS];
  const options = keys.map((key) => ({
    value: key,
    label: keyName(key),
    disabled: Boolean(taken[key]),
    hint: taken[key] ? t(taken[key]) : ownkeyKeys.includes(key) ? t('settings.keys.ownkey') : ''
  }));
  const value = await choose({ title: t(KEY_TITLES[role]), options, current: getPath(c, role), note: state.devMode ? t('settings.keys.dev') : '' });
  if (value != null && value !== getPath(c, role)) setValue(role, value);
}

async function pairChoice(key, title, pairs) {
  const value = await choose({ title, options: pairs.map(([v, label]) => ({ value: v, label: t(label) })), current: getPath(state.config, key) });
  if (value != null && value !== getPath(state.config, key)) setValue(key, value);
}

const CHOICES = {
  'dictation.hotkey': () => keyChoice('dictation.hotkey'),
  'dictation.rewriteHotkey': () => keyChoice('dictation.rewriteHotkey'),
  'meetings.stopKey': () => keyChoice('meetings.stopKey'),
  'ui.pillEdge': () => pairChoice('ui.pillEdge', t('settings.pill.edge'), EDGES),
  'meetings.retention': () => pairChoice('meetings.retention', t('settings.meetings.retention'), RETENTION),
  'rewriting.tone': () => pairChoice('rewriting.tone', t('settings.rewriting.tone'), TONES),
  'ui.theme': () => pairChoice('ui.theme', t('settings.general.theme'), THEMES),
  'ui.language': async () => {
    const options = [{ value: 'system', label: t('common.system') }, ...UI_LANGUAGES.map(([value, label]) => ({ value, label }))];
    const value = await choose({ title: t('settings.general.language'), options, current: state.config.ui.language });
    if (value != null && value !== state.config.ui.language) setValue('ui.language', value);
  }
};

function draft(form) {
  return (drafts[form] ??= {});
}

function providerChanged(target, provider) {
  const d = draft(target);
  d.provider = provider;
  d.apiKey = '';
  d.replacingKey = false;
  d.endpoint = provider === 'orukeet' ? '' : presetEndpoints(target, provider)[0] || '';
  d.model = provider === 'orukeet' ? ORUKEET_MODEL : '';
  delete modelLists[target];
  delete formStatus[target];
  render();
}

function validateProvider(target, values) {
  if (values.provider === 'orukeet') return null;
  if (values.endpoint && values.model) return null;
  return { rewriting: 'settings.provider.needRewrite', meetings: 'settings.provider.needMeeting' }[target] || 'settings.provider.needAudio';
}

async function saveProvider(target) {
  const saved = state.config[target];
  const d = drafts[target] || {};
  const provider = formValue(target, 'provider');
  const local = provider === 'orukeet';
  const values = {
    target,
    provider,
    endpoint: local ? '' : String(formValue(target, 'endpoint')).trim(),
    model: local ? ORUKEET_MODEL : String(formValue(target, 'model')).trim()
  };
  const problem = validateProvider(target, values);
  if (problem) {
    formStatus[target] = { kind: 'invalid', message: t(problem) };
    render();
    return;
  }
  // null keeps the saved key, "" saves none, text replaces it. The app never keeps a key across providers.
  const typed = String(d.apiKey || '').trim();
  const showingSaved = saved.keySaved && provider === saved.provider && !d.replacingKey;
  values.apiKey = local ? '' : typed ? typed : showingSaved || (saved.keySaved && provider === saved.provider) ? null : '';
  if (target === 'transcription') values.language = local ? 'auto' : formValue('transcription', 'language') || 'auto';
  if (target === 'meetings') values.consent = consentNeeded(provider, values.endpoint) ? consentValue(provider, values.endpoint, values.model) : false;
  formStatus[target] = { kind: local ? 'loading' : 'saving', message: t(local ? 'settings.provider.loadingOrukeet' : 'settings.provider.saving') };
  render();
  try {
    await invoke('settings_save_provider', { save: values });
    delete drafts[target];
    formStatus[target] = { kind: 'saved', message: t('settings.provider.saved') };
  } catch (error) {
    formStatus[target] = { kind: 'failed', message: t('settings.provider.failed'), detail: error?.message || String(error) };
  }
  render();
}

// Ownkey's rule for Refresh: a list that arrives after the provider, endpoint or key changed is dropped.
async function refreshModels(target) {
  const provider = formValue(target, 'provider');
  const endpoint = String(formValue(target, 'endpoint')).trim();
  const typed = String(drafts[target]?.apiKey || '').trim();
  const asked = `${provider}|${endpoint}|${typed}`;
  modelLists[target] = { status: 'loading', list: modelLists[target]?.list || [], for: asked };
  render();
  const still = () => modelLists[target]?.for === asked && `${formValue(target, 'provider')}|${String(formValue(target, 'endpoint')).trim()}|${String(drafts[target]?.apiKey || '').trim()}` === asked;
  try {
    const models = await invoke('settings_list_models', { request: { target, provider, endpoint, apiKey: typed || null } });
    if (!still()) return;
    modelLists[target] = { status: 'done', list: models, for: asked };
  } catch (error) {
    if (!still()) return;
    modelLists[target] = { status: 'failed', list: [], error: error?.message || String(error), for: asked };
  }
  render();
}

function normalize(text) {
  return String(text || '').replace(/\s+/g, ' ').trim();
}

// Ownkey's dictionary rules: whitespace collapsed, and an entry with the same word or misspelling, ignoring case,
// is replaced rather than duplicated.
function addEntry() {
  const { vocabulary, corrections } = state.config.dictionary;
  const d = draft('dict');
  if (dictionary.correcting) {
    const from = normalize(d.from);
    const to = normalize(d.to);
    if (!from || !to) return dictionaryError('settings.dictionary.needBoth');
    if (from === to) return dictionaryError('settings.dictionary.identical');
    const next = corrections.filter((c) => c.from.toLowerCase() !== from.toLowerCase()).concat({ from, to });
    d.from = '';
    d.to = '';
    dictionary.error = '';
    setValue('dictionary.corrections', next).then(() => focusField('dict.from'));
  } else {
    const word = normalize(d.word);
    if (!word) return dictionaryError('settings.dictionary.needWord');
    const next = vocabulary.filter((w) => w.toLowerCase() !== word.toLowerCase()).concat(word);
    d.word = '';
    dictionary.error = '';
    setValue('dictionary.vocabulary', next).then(() => focusField('dict.word'));
  }
}

function dictionaryError(key) {
  dictionary.error = t(key);
  render();
}

function removeEntry(kind, value) {
  const { vocabulary, corrections } = state.config.dictionary;
  if (kind === 'word') setValue('dictionary.vocabulary', vocabulary.filter((w) => w !== value));
  else setValue('dictionary.corrections', corrections.filter((c) => c.from !== value));
}

function focusField(field) {
  page.querySelector(`[data-field="${field}"]`)?.focus();
}

let previewTimer = 0;
function schedulePreview() {
  clearTimeout(previewTimer);
  previewTimer = setTimeout(async () => {
    const f = state.config.fillers;
    const custom = formValue('fillers', 'custom');
    const asked = JSON.stringify([f.enabled, f.languages, custom]);
    try {
      const text = await invoke('settings_clean_preview', { preview: { text: FILLER_SAMPLE, enabled: f.enabled, languages: f.languages, custom } });
      if (asked !== JSON.stringify([state.config.fillers.enabled, state.config.fillers.languages, formValue('fillers', 'custom')])) return;
      preview = { text, for: asked };
      render();
    } catch {
      // The preview is a nicety; the saved setting is what counts.
    }
  }, 150);
}

// ---------- actions ----------

async function act(action, button) {
  const [name, ...rest] = action.split(':');
  const arg = rest.join(':');
  switch (name) {
    case 'back':
      return back();
    case 'go':
      return go(arg);
    case 'toggle':
      if (arg === 'fillers.enabled') schedulePreview();
      if (arg === 'storage.sync.enabled' && !state.config.storage.sync.enabled) return turnSyncOn();
      return setValue(arg, !getPath(state.config, arg));
    case 'choose':
      return CHOICES[arg]?.();
    case 'seg': {
      const at = arg.lastIndexOf(':');
      return setValue(arg.slice(0, at), arg.slice(at + 1));
    }
    case 'filler-language': {
      const languages = state.config.fillers.languages;
      const next = languages.includes(arg) ? languages.filter((code) => code !== arg) : [...languages, arg];
      schedulePreview();
      return setValue('fillers.languages', next);
    }
    case 'save':
      return saveProvider(arg);
    case 'refresh':
      return refreshModels(arg);
    case 'replace-key':
      draft(arg).replacingKey = true;
      render();
      return focusField(`${arg}.apiKey`);
    case 'remove-key':
      try {
        await invoke('settings_remove_key', { target: arg });
        state.config[arg].keySaved = false;
        render();
      } catch (error) {
        await showError(t('settings.errors.failed'), error?.message);
      }
      return;
    case 'details':
      return showError(formStatus[arg]?.message || '', formStatus[arg]?.detail);
    case 'speakers-details':
      return showError(speakersReason() || '', state.speakers?.detail);
    case 'model': {
      const [id, what] = arg.split(':');
      const m = state.models[id];
      if (what === 'details') return showError(modelStatus(m).text, m.error);
      if (what === 'remove') {
        const ok = await dialog({
          title: t('settings.model.removeTitle', { model: m.name }),
          body: `<p>${esc(t('settings.model.removeBody'))}</p>`,
          buttons: [
            { label: t('common.cancel'), value: false },
            { label: t('settings.model.remove'), value: true, primary: true, danger: true }
          ]
        });
        if (!ok) return;
      }
      try {
        await invoke('settings_model', { model: id, action: what });
      } catch (error) {
        await showError(t('settings.errors.failed'), error?.message);
      }
      return;
    }
    case 'folder':
      try {
        await invoke('settings_choose_folder', { purpose: arg });
      } catch (error) {
        await showError(t('settings.errors.failed'), error?.message);
      }
      return;
    case 'folder-reset':
      return setValue('local.modelFolder', '');
    case 'open':
      return invoke('settings_open_folder', { folder: arg }).catch((error) => showError(t('settings.errors.failed'), error?.message));
    case 'link':
      return invoke('settings_open_link', { link: arg }).catch(() => {});
    case 'dict-mode':
      dictionary = { correcting: !dictionary.correcting, error: '' };
      render();
      return focusField(dictionary.correcting ? 'dict.from' : 'dict.word');
    case 'dict-add':
      return addEntry();
    case 'dict-remove': {
      const [kind, value] = arg.split(':');
      return removeEntry(kind, decodeURIComponent(value));
    }
    case 'updates-check':
      state.updates = { ...state.updates, state: 'checking' };
      render();
      try {
        state.updates = await invoke('settings_check_updates');
      } catch {
        state.updates = { state: 'failed', version: null };
      }
      return render();
    case 'new-token':
      button.disabled = true;
      try {
        await invoke('settings_new_token');
        tokenDone = true;
      } catch (error) {
        await showError(t('settings.errors.failed'), error?.message);
      }
      return render();
    case 'import':
      return runImport();
    case 'licenses':
      return licensesDialog();
    case 'sync-test':
      return testSync();
    case 'sync-details': {
      if (arg === 'test') return showError(t(SYNC_ERRORS[syncTest?.code] || 'settings.sync.error.failed'), syncTest?.message);
      if (arg === 'restore') return showError(t('settings.sync.restoreFailed'), state.sync?.restore?.error);
      if (arg === 'unavailable') return showError(t('settings.sync.unavailable'), state.sync?.lastError?.message);
      // As in the status line, `failed` says "Uploads stopped." rather than "The test failed.".
      const last = state.sync?.lastError;
      return showError(t((last?.code !== 'failed' && SYNC_ERRORS[last?.code]) || 'settings.sync.stopped'), last?.message);
    }
    case 'sync-secret-replace':
      syncReplacing = true;
      dropSyncTest();
      render();
      return focusField('storage.sync.secret');
    case 'sync-secret-remove':
      button.disabled = true;
      try {
        await trackSync(invoke('settings_remove_key', { target: 'sync' }));
        secretVersion += 1;
        dropSyncTest();
        Object.assign(state.config.storage.sync, { secretSaved: false, tested: false });
      } catch (error) {
        await showError(t('settings.errors.failed'), error?.message);
      }
      return render();
    case 'sync-key-show':
      return keyFlow('show');
    case 'sync-key-use':
      return keyFlow('use');
    case 'sync-restore':
      // Progress arrives in sync.restore; the button goes with the next state.
      button.disabled = true;
      try {
        await invoke('settings_sync_restore', { action: 'start' });
      } catch (error) {
        button.disabled = false;
        await showError(t('settings.sync.restoreFailed'), error?.message);
      }
      return null;
  }
  return null;
}

// ---------- bucket sync: secret and Test connection ----------

// Remembers a settings_set or secret call for the storage.sync fields, so Test connection and turning sync on run after
// it: a press on either blurs the field first, and they must see the value that was just typed. Each call settles to
// whether it saved, so they don't run when it didn't (syncSaved).
function trackSync(promise) {
  const settled = promise.then(
    (saved) => saved !== false,
    () => false
  );
  syncPending.add(settled);
  settled.then(() => syncPending.delete(settled));
  return promise;
}

// Waits for the storage.sync calls still running. Resolves to whether each saved and no field shows a refused value.
async function syncSaved() {
  const saved = await Promise.all([...syncPending]);
  return saved.every(Boolean) && !syncRefused.size;
}

// Turning sync on, which waits for the fields being saved (see trackSync).
async function turnSyncOn() {
  if (!(await syncSaved())) return render();
  return setValue('storage.sync.enabled', true);
}

// The bucket's secret goes to settings_sync_save_secret when its field loses focus, never through settings_set. The
// page keeps no copy: the field and its draft are emptied before the call, also when the call fails.
function saveSyncSecret(input) {
  const secret = input.value.trim();
  input.value = '';
  if (drafts.storage) delete drafts.storage['sync.secret'];
  if (!secret) return null;
  return trackSync(
    (async () => {
      try {
        await invoke('settings_sync_save_secret', { secret });
        secretVersion += 1;
        syncReplacing = false;
        Object.assign(state.config.storage.sync, { secretSaved: true, tested: false });
        render();
        return true;
      } catch (error) {
        // Test connection goes back to waiting for a secret. Not before the call: this runs from the field's change
        // event, and a click on Test that took the focus must still land on the button it pressed.
        syncTestButton();
        await showError(t('settings.errors.failed'), error?.message);
        return false;
      }
    })()
  );
}

// Test connection turns on with the first character of a secret and off when its field is emptied again: clicking Test
// takes the focus from the field, which saves the secret, and the test waits for that save (testSync). In place, not
// through the markup: a redraw while the secret is typed would take the focused field out of the page, and Chromium
// then commits it as on blur, half-typed.
function syncTestButton() {
  const test = page.querySelector('[data-a="sync-test"]');
  const typed = page.querySelector('[data-field="storage.sync.secret"]')?.value.trim();
  if (test) test.disabled = currentSyncTest()?.kind === 'testing' || syncRefused.size > 0 || (!state.config.storage.sync.secretSaved && !typed);
}

// The secret is being replaced or removed, so a test still running is about the old one. Its answer would also redraw
// the card under the new secret's field while it is typed (see syncTestButton), so it is dropped when it comes.
function dropSyncTest() {
  syncTestRun += 1;
  syncTest = null;
}

async function testSync() {
  if (syncRefused.size) return;
  const run = ++syncTestRun;
  syncTest = { kind: 'testing', for: syncConnectionKey() };
  render();
  const saved = await syncSaved();
  if (run !== syncTestRun) return;
  // A value the pressed button saved was refused: the test would run with the saved one, which the form doesn't show.
  if (!saved) {
    syncTest = null;
    render();
    return;
  }
  const asked = syncConnectionKey();
  syncTest = { kind: 'testing', for: asked };
  render();
  let result;
  try {
    const report = await invoke('settings_sync_test');
    result = { kind: 'ok', for: asked, otherKeys: report?.otherKeys ?? 0 };
    // The app marked these fields as tested before it answered; its state says so too.
    if (asked === syncConnectionKey()) state.config.storage.sync.tested = true;
  } catch (error) {
    result = { kind: 'failed', for: asked, code: error?.code, message: error?.message || String(error) };
  }
  // A test that started after this one has the say.
  if (run !== syncTestRun) return;
  syncTest = result;
  render();
}

// ---------- bucket sync: the encryption key ----------

// The recovery code never shows in a screen recording or a screen share (RUK-110 D18). While a key dialog is open,
// the Settings window is hidden from capture (settings_protect); a meeting that starts recording the screen closes
// the dialog; and the code lives only in the dialog's DOM, which is emptied when it closes. Copy goes through the
// app, which keeps the code out of clipboard history and the cloud clipboard; the web clipboard API would put it in
// both, so the page never touches it (dev/check.js checks).
async function keyFlow(kind) {
  if (keyDialog) return;
  const title = t(kind === 'show' ? 'settings.sync.keyTitle' : 'settings.sync.keyUse');
  if (state.sync?.screenRecording) return notice(title, t('settings.sync.keyScreen'));
  // Taken before the first await, so a second click can't open another dialog.
  keyDialog = () => {};
  try {
    await invoke('settings_protect', { on: true });
  } catch (error) {
    keyDialog = null;
    return showError(t('settings.sync.keyProtect'), error?.message);
  }
  let outcome = null;
  try {
    outcome = kind === 'show' ? await showKeyDialog() : await useKeyDialog();
  } finally {
    keyDialog = null;
    await invoke('settings_protect', { on: false }).catch(() => {});
  }
  if (outcome === 'screen') return notice(title, t('settings.sync.keyScreen'));
  if (outcome?.error) return showError(outcome.message, outcome.error.message);
  return null;
}

async function showKeyDialog() {
  let code;
  try {
    ({ code } = await invoke('settings_sync_key', { action: 'show' }));
  } catch (error) {
    if (error?.code === 'recording_screen') return 'screen';
    // Without a key, show makes one, so its failure isn't an unreadable key; "use your saved key" would mislead.
    return { error, message: t(state.sync?.key?.exists ? 'settings.sync.keyUnreadable' : 'settings.errors.failed') };
  }
  if (state.sync?.screenRecording) return 'screen';
  const value = await dialog({
    title: t('settings.sync.keyTitle'),
    body: `<code class="recovery-key"></code>
      <div class="recovery-actions"><button class="btn sm secondary" data-copy>${icons.copy}<span>${esc(t('settings.sync.keyCopy'))}</span></button>
        <span class="dialog-error" role="alert" hidden></span></div>
      <p>${esc(t('settings.sync.keyHelp'))}</p>`,
    buttons: [
      { label: t('common.close'), value: null },
      { label: t('settings.sync.keyDone'), value: 'saved', primary: true }
    ],
    wipe: true,
    onRender(body, done) {
      keyDialog = () => done('screen');
      // The state may have said so while the code was on its way.
      if (state.sync?.screenRecording) return done('screen');
      // The code goes in as text, with a line break allowed only after each group's dash.
      const box = body.querySelector('.recovery-key');
      const groups = String(code).split('-');
      groups.forEach((group, i) => {
        if (i < groups.length - 1) box.append(`${group}-`, document.createElement('wbr'));
        else box.append(group);
      });
      code = null;
      const copy = body.querySelector('[data-copy]');
      const problem = body.querySelector('.dialog-error');
      copy.addEventListener('click', async () => {
        copy.disabled = true;
        try {
          await invoke('settings_sync_key_copy');
          copy.innerHTML = `${icons.check}<span>${esc(t('settings.sync.keyCopied'))}</span>`;
          problem.hidden = true;
        } catch {
          problem.textContent = t('settings.sync.keyCopyFailed');
          problem.hidden = false;
        }
        copy.disabled = false;
      });
    }
  });
  code = null;
  if (value === 'saved') {
    try {
      await invoke('settings_sync_key', { action: 'saved' });
      state.sync.key = { ...state.sync.key, exported: true };
      render();
    } catch (error) {
      await showError(t('settings.errors.failed'), error?.message);
    }
  }
  return value === 'screen' ? 'screen' : null;
}

async function useKeyDialog() {
  const value = await dialog({
    title: t('settings.sync.keyUse'),
    body: `<p>${esc(t('settings.sync.keyUseHelp'))}</p>
      <textarea class="recovery-input" rows="3" spellcheck="false" autocomplete="off" autocapitalize="off" aria-label="${esc(t('settings.sync.keyUse'))}"></textarea>
      <p class="dialog-error" role="alert" hidden></p>`,
    buttons: [
      { label: t('common.cancel'), value: null },
      { label: t('settings.sync.keyUseButton'), primary: true, run: importKey }
    ],
    wipe: true,
    onRender(body, done) {
      keyDialog = () => done('screen');
      if (state.sync?.screenRecording) return done('screen');
      body.querySelector('textarea').focus();
    }
  });
  if (value && typeof value === 'object') {
    state.sync.key = { exists: true, id: value.id ?? state.sync.key?.id ?? null, exported: true };
    render();
  }
  return value === 'screen' ? 'screen' : null;
}

// "Use this key": the dialog stays open on a code the app refuses and says why.
async function importKey(done, button) {
  const scrim = button.closest('.scrim');
  const input = scrim.querySelector('.recovery-input');
  const problem = scrim.querySelector('.dialog-error');
  const refuse = (key) => {
    problem.textContent = t(key);
    problem.hidden = false;
    input.focus();
  };
  const code = input.value.trim();
  if (!code) return refuse('settings.sync.keyInvalid');
  button.disabled = true;
  try {
    const answer = await invoke('settings_sync_key', { action: 'import', code });
    return done(answer || {});
  } catch (error) {
    // A meeting records the screen: the dialog closes, as it does when the state says so first.
    if (error?.code === 'recording_screen') return done('screen');
    button.disabled = false;
    return refuse(KEY_ERRORS[error?.code] || 'settings.errors.failed');
  }
}

function notice(title, message) {
  return dialog({ title, body: `<p>${esc(message)}</p>`, buttons: [{ label: t('common.close'), value: true, primary: true }] });
}

async function runImport() {
  if (state.ownkey.lastImport) {
    const ok = await dialog({
      title: t('settings.import.confirmTitle'),
      body: `<p>${esc(t('settings.import.confirmBody'))}</p><p>${esc(t('settings.import.readOnly'))}</p>`,
      buttons: [
        { label: t('common.cancel'), value: false },
        { label: t('settings.import.again'), value: true, primary: true }
      ]
    });
    if (!ok) return;
  }
  importStatus = { kind: 'running' };
  render();
  try {
    await invoke('settings_import_ownkey');
    importStatus = null;
  } catch (error) {
    importStatus = { kind: 'failed', message: error?.message || String(error) };
  }
  render();
}

function licensesDialog() {
  const items = [
    esc(t('settings.licenses.inter')),
    `${esc(t('settings.licenses.orukeet'))} <button class="link-btn" data-link="orukeet">${icons.link}huggingface.co/oruk/orukeet</button>`,
    feature('speakers') ? `${esc(t('settings.licenses.nemotron'))} <button class="link-btn" data-link="nemotron">${icons.link}huggingface.co/nvidia</button>` : '',
    esc(t('settings.licenses.beautifulUi'))
  ].filter(Boolean);
  return dialog({
    title: t('settings.about.licenses'),
    body: `<ul class="license-list">${items.map((item) => `<li>${item}</li>`).join('')}</ul>`,
    buttons: [{ label: t('common.close'), value: true, primary: true }],
    onRender(body) {
      body.querySelectorAll('[data-link]').forEach((b) => b.addEventListener('click', () => invoke('settings_open_link', { link: b.dataset.link }).catch(() => {})));
    }
  });
}

function showError(message, detail) {
  return dialog({
    title: t('common.errorDetails'),
    body: `${message ? `<p>${esc(message)}</p>` : ''}${detail ? `<div class="detail">${esc(detail)}</div>` : ''}`,
    buttons: [{ label: t('common.close'), value: true, primary: true }]
  });
}

// Rukoo's dialog: a scrim, Escape and a click outside cancel, Tab stays inside. A button with `run` doesn't close the
// dialog; run(done, button) decides. `wipe` empties the dialog's DOM when it closes, for what mustn't linger.
function dialog({ title, body, buttons, onRender, wipe = false }) {
  return new Promise((resolve) => {
    const before = document.activeElement;
    const scrim = document.createElement('div');
    scrim.className = 'scrim';
    scrim.innerHTML = `<div class="dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title">
      <h2 id="dialog-title">${esc(title)}</h2><div class="dialog-body">${body}</div>
      <div class="buttons">${buttons
        .map((b, i) => `<button class="btn ${b.primary ? 'primary' : 'secondary'} ${b.danger && !b.primary ? 'danger' : ''}" data-i="${i}">${esc(b.label)}</button>`)
        .join('')}</div></div>`;
    let closed = false;
    const done = (value) => {
      if (closed) return;
      closed = true;
      if (wipe) scrim.replaceChildren();
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
      if (e.key === 'Tab') {
        const items = [...scrim.querySelectorAll('button, input, select, textarea')].filter((x) => !x.disabled);
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
    scrim.addEventListener('mousedown', (e) => e.target === scrim && done(null));
    scrim.querySelectorAll('.buttons button').forEach((b) =>
      b.addEventListener('click', () => {
        const button = buttons[Number(b.dataset.i)];
        if (button.run) button.run(done, b);
        else done(button.value);
      })
    );
    document.body.append(scrim);
    if (onRender) onRender(scrim.querySelector('.dialog-body'), done);
    if (!scrim.contains(document.activeElement)) scrim.querySelector('.buttons .btn.primary, .buttons .btn:last-child')?.focus();
  });
}

// ---------- navigation ----------

function go(name, { anchor } = {}) {
  if (!PAGE_BUILDERS[name]) return;
  if (name !== current()) stack.push(name);
  dictionary.error = '';
  render({ full: true });
  page.classList.remove('entering');
  void page.offsetWidth;
  page.classList.add('entering');
  if (anchor) page.querySelector(`#group-${anchor}`)?.scrollIntoView({ block: 'start' });
}

function back() {
  if (stack.length > 1) {
    // Leaving a provider form without Save discards it, like Ownkey's Cancel.
    const leaving = stack.pop();
    const form = { transcription: 'transcription', rewriting: 'rewriting', meetingTranscription: 'meetings' }[leaving];
    if (form) {
      delete drafts[form];
      delete formStatus[form];
      delete modelLists[form];
    }
    // Leaving Storage starts its forms afresh next time. A field focused as the page goes still saves, as on blur.
    if (leaving === 'storage') {
      delete drafts.storage;
      syncRefused.clear();
      syncReplacing = false;
      syncTest = null;
    }
    render({ full: true });
    return;
  }
  invoke('settings_close').catch(() => {});
}

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && !document.querySelector('.scrim')) {
    e.preventDefault();
    back();
  }
  if (e.key === 'Enter' && e.target.matches?.('[data-field^="dict."]')) {
    e.preventDefault();
    addEntry();
  }
});

page.addEventListener('click', (e) => {
  const target = e.target.closest('[data-a]');
  if (!target || target.disabled || !page.contains(target)) return;
  act(target.dataset.a, target);
});

page.addEventListener('input', (e) => {
  const field = e.target.dataset?.field;
  if (!field) return;
  const [form, ...rest] = field.split('.');
  const name = rest.join('.');
  if (e.target.type === 'checkbox') return;
  draft(form)[name] = e.target.value;
  if (form === 'local' && name === 'idleMinutes') {
    const minutes = Number(e.target.value);
    const bad = e.target.value === '' || !Number.isFinite(minutes) || minutes < 0 || minutes > 1440;
    if (bad !== idleError) {
      idleError = bad;
      render();
    }
  }
  if (form === 'fillers' && name === 'custom') schedulePreview();
  if (field === 'storage.sync.secret') syncTestButton();
});

page.addEventListener('change', (e) => {
  const field = e.target.dataset?.field;
  if (!field) return;
  const [form, ...rest] = field.split('.');
  const name = rest.join('.');
  if (field === 'meetings.consent') {
    const d = draft('meetings');
    d.consent = e.target.checked;
    d.consentFor = consentKey(formValue('meetings', 'provider'), formValue('meetings', 'endpoint'), formValue('meetings', 'model'));
    render();
    return;
  }
  // Fields that save on their own when they lose focus.
  const key = `${form}.${name}`;
  // The bucket secret never goes through settings_set.
  if (key === 'storage.sync.secret') return saveSyncSecret(e.target);
  if (key.startsWith('storage.sync.')) {
    const value = e.target.value;
    delete drafts[form]?.[name];
    // A refused value, such as an http:// endpoint, stays in the field to be fixed. Test connection and turning sync on
    // wait until a value saves there (syncConnectionCard).
    return trackSync(
      setValue(key, value).then((ok) => {
        if (ok) syncRefused.delete(name);
        else {
          draft(form)[name] = value;
          syncRefused.add(name);
        }
        render();
        return ok;
      })
    );
  }
  if (key === 'fillers.custom' || key === 'rewriting.instructions') {
    const value = e.target.value;
    delete drafts[form]?.[name];
    return setValue(key, value);
  }
  if (PROVIDER_FORMS.includes(form)) {
    if (name === 'provider') return providerChanged(form, e.target.value);
    draft(form)[name] = e.target.value;
    delete formStatus[form];
    // The consent text names the endpoint's host, so it follows the endpoint.
    return render();
  }
  if (key === 'local.idleMinutes') {
    if (idleError) return;
    delete drafts.local;
    return setValue(key, Math.round(Number(e.target.value)));
  }
});

// ---------- state from the app ----------

function applyState(next, { first = false } = {}) {
  // Each state has a seq of its own (API.md), so one that isn't newer is stale. That holds for the first
  // settings_state answer too: an event can arrive before it, with a newer state.
  if (!next || (state && next.seq <= state.seq)) return;
  // The first state shown, from either, draws the whole page.
  const full = first || !state;
  const relocalize = full || next.locale !== state.locale;
  const retheme = full || next.theme !== state.theme;
  state = next;
  if (retheme) {
    const root = document.documentElement;
    root.classList.add('theme-switching');
    root.classList.toggle('light', next.theme === 'light');
    requestAnimationFrame(() => requestAnimationFrame(() => root.classList.remove('theme-switching')));
  }
  if (relocalize) {
    setLanguage(next.locale);
    localize(document);
  }
  render({ full: relocalize });
  // A meeting started recording the screen, from the pill, a key or Rukoo: a key dialog closes (RUK-110 D18).
  if (keyDialog && next.sync?.screenRecording) keyDialog();
}

async function start() {
  await connect('settings');
  await loadCatalogs();
  await listen('settings://state', (next) => applyState(next));
  applyState(await invoke('settings_state'), { first: true });
  // ?page=<page> opens a sub-page, ?page=<group> scrolls the top page to a group (status, dictation, meetings,
  // general, about). The app uses it to open Settings at the Ownkey import, for example.
  const wanted = params.get('page');
  if (wanted === 'meetingtranscription' || wanted === 'meeting-transcription') go('meetingTranscription');
  else if (PAGES.includes(wanted) && wanted !== 'main') go(wanted);
  else if (GROUP_ANCHORS.includes(wanted)) page.querySelector(`#group-${wanted}`)?.scrollIntoView({ block: 'start' });
  schedulePreview();
}

start().catch((error) => {
  console.error('settings: failed to start', error);
});
