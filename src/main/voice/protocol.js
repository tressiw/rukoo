'use strict';

// The bridge protocol with Rukoo Voice, version 2: voice/PROTOCOL.md is the contract, and
// voice/fixtures/protocol-vectors.json holds the examples both sides test against. Pure, no Electron.
//
// validate() implements "Checking messages". A message that breaks a rule is dropped; the result names the field,
// as the fixture's "reason" does, so both sides can be held to the same examples.

const PROTOCOL = 2;
const MIN_PROTOCOL = 1;

// Voice sends at most 8 MiB in one message, Rukoo at most 1 MiB.
const VOICE_MESSAGE_MAX = 8 * 1024 * 1024;
const RUKOO_MESSAGE_MAX = 1024 * 1024;
const DISCOVERY_MAX = 4096;
const CONTEXT_MAX = 16 * 1024;
const TITLE_MAX = 200;
// A mark's label, like a title. PROTOCOL.md leaves it open; the shared vectors settle it.
const LABEL_MAX = 200;

const MEETING_ID = /^m-[0-9]{8}-[0-9]{6}-[0-9a-f]{4}$/;
const TOKEN = /^[A-Za-z0-9_-]{43}$/;
const INSTANCE = /^[A-Za-z0-9_-]{22}$/;

const TRACKS = ['mic', 'call'];
const SCREENS = ['off', 'frames', 'video'];
const STATES = ['recording', 'paused', 'processing', 'done', 'failed'];
const ENDED = new Set(['processing', 'done', 'failed']);
const WITH_REASON = new Set(['processing', 'failed']);
const MARK_TRACKS = ['mic', 'call', 'screen'];
const MEETING_EVENTS = new Set(['meeting.started', 'meeting.starting', 'meeting.state', 'speaker', 'passage', 'passage.removed', 'mark', 'frame', 'meeting.removed']);
const ERROR_CODES = ['bad_request', 'unknown_command', 'not_found', 'not_recording', 'busy', 'not_available', 'failed'];

// media.open and media.read: what Voice prepares, its ids, the longest range and the most bytes one read returns.
const MEDIA_KINDS = ['audio', 'frame', 'video'];
const MEDIA_ID = /^md-[A-Za-z0-9_-]{22}$/;
const MEDIA_SPAN_MAX = 3600000;
const MEDIA_READ_MAX = 1024 * 1024;

class Invalid extends Error {
  constructor(reason) {
    super(`invalid ${reason}`);
    this.reason = reason;
  }
}

const isObj = (v) => Boolean(v) && typeof v === 'object' && !Array.isArray(v);
const has = (o, k) => Object.prototype.hasOwnProperty.call(o, k) && o[k] !== undefined;
// An optional field that is null counts as left out, as Voice reads it (voice/fixtures/protocol-vectors.json).
const given = (o, k) => has(o, k) && o[k] !== null;

function check(ok, reason) {
  if (!ok) throw new Invalid(reason);
}

// "Integers are JSON numbers without a fraction."
function int(o, k, min = 0) {
  check(has(o, k) && Number.isInteger(o[k]) && o[k] >= min, k);
  return o[k];
}

// Lengths count characters (code points), as Voice counts them, not UTF-16 units: an emoji is one character. Counts
// no further than one past `upTo`, so a long passage isn't walked for a short limit.
function chars(s, upTo) {
  let n = 0;
  for (const _ of s) if (++n > upTo) break;
  return n;
}

function str(o, k, { min = 0, max = Infinity, nullable = false } = {}) {
  if (nullable && has(o, k) && o[k] === null) return null;
  check(has(o, k) && typeof o[k] === 'string', k);
  const s = o[k];
  check((min === 0 || chars(s, min) >= min) && (max === Infinity || chars(s, max) <= max), k);
  return o[k];
}

function oneOf(o, k, values, { nullable = false } = {}) {
  if (nullable && has(o, k) && o[k] === null) return null;
  check(has(o, k) && values.includes(o[k]), k);
  return o[k];
}

function pattern(o, k, re, { nullable = false } = {}) {
  if (nullable && has(o, k) && o[k] === null) return null;
  check(has(o, k) && typeof o[k] === 'string' && re.test(o[k]), k);
  return o[k];
}

function bool(o, k) {
  check(has(o, k) && typeof o[k] === 'boolean', k);
  return o[k];
}

function tracks(o, k = 'tracks') {
  const v = o[k];
  check(Array.isArray(v) && v.length > 0 && v.every((t) => TRACKS.includes(t)) && new Set(v).size === v.length, k);
  return v;
}

function status(o) {
  pattern(o, 'recording', MEETING_ID, { nullable: true });
  bool(o, 'dictating');
  // The screen a meeting gets when meeting.start leaves it out. An older Voice leaves it out.
  if (given(o, 'screenDefault')) oneOf(o, 'screenDefault', SCREENS);
}

// ---------- messages from Rukoo ----------

function hello(m) {
  int(m, 'protocol', 1);
  pattern(m, 'token', TOKEN);
  str(m, 'client', { min: 1, max: 64 });
  pattern(m, 'instance', INSTANCE, { nullable: true });
  int(m, 'after', 0);
  if (given(m, 'clientVersion')) str(m, 'clientVersion');
  // Rukoo's process id, so Voice can leave its windows out of a screen recording.
  if (given(m, 'pid')) int(m, 'pid', 1);
}

const COMMAND_ARGS = {
  'meeting.start': (a) => {
    if (given(a, 'title')) str(a, 'title', { max: TITLE_MAX });
    if (given(a, 'tracks')) tracks(a);
    if (given(a, 'screen')) oneOf(a, 'screen', SCREENS);
    // 16 KiB of compact JSON in UTF-8, as Voice measures it.
    if (given(a, 'context')) check(isObj(a.context) && Buffer.byteLength(JSON.stringify(a.context), 'utf8') <= CONTEXT_MAX, 'context');
  },
  'meeting.stop': (a) => pattern(a, 'meeting', MEETING_ID),
  'meeting.delete': (a) => pattern(a, 'meeting', MEETING_ID),
  'meeting.pause': (a) => pattern(a, 'meeting', MEETING_ID),
  'meeting.resume': (a) => pattern(a, 'meeting', MEETING_ID),
  'meeting.mark': (a) => {
    pattern(a, 'meeting', MEETING_ID);
    if (given(a, 'label')) str(a, 'label', { max: LABEL_MAX });
  },
  'meeting.list': (a) => {
    if (given(a, 'limit')) int(a, 'limit', 1);
  },
  'meeting.get': (a) => pattern(a, 'meeting', MEETING_ID),
  'frame.get': (a) => {
    pattern(a, 'meeting', MEETING_ID);
    int(a, 'at', 0);
  },
  'status.get': () => {},
  'clip.get': (a) => {
    pattern(a, 'meeting', MEETING_ID);
    const start = int(a, 'start', 0);
    const end = int(a, 'end', 0);
    check(end >= start, 'end');
  },
  'meeting.protected': (a) => {
    pattern(a, 'meeting', MEETING_ID);
    if (given(a, 'request')) str(a, 'request', { min: 1, max: 64 });
  },
  'screen.set': (a) => {
    pattern(a, 'meeting', MEETING_ID);
    oneOf(a, 'screen', SCREENS);
    if (given(a, 'monitor')) str(a, 'monitor', { min: 1, max: 64 });
  },
  'screen.get': (a) => pattern(a, 'meeting', MEETING_ID),
  'screen.select': (a) => {
    pattern(a, 'meeting', MEETING_ID);
    str(a, 'monitor', { min: 1, max: 64 });
  },
  'media.open': (a) => {
    pattern(a, 'meeting', MEETING_ID);
    oneOf(a, 'kind', MEDIA_KINDS);
    int(a, 'start', 0);
    if (given(a, 'end')) {
      int(a, 'end', 0);
      check(a.end >= a.start && a.end - a.start <= MEDIA_SPAN_MAX, 'end');
    }
  },
  'media.read': (a) => {
    pattern(a, 'media', MEDIA_ID);
    int(a, 'offset', 0);
    int(a, 'length', 1);
    check(a.length <= MEDIA_READ_MAX, 'length');
  }
};

function command(m) {
  str(m, 'id', { min: 1, max: 64 });
  str(m, 'command', { min: 1 });
  if (!given(m, 'args')) m = { ...m, args: {} };
  check(isObj(m.args), 'args');
  const args = COMMAND_ARGS[m.command];
  if (args) args(m.args);
  return m;
}

// ---------- messages from Voice ----------

function welcome(m) {
  int(m, 'protocol', 1);
  str(m, 'voiceVersion');
  pattern(m, 'instance', INSTANCE);
  int(m, 'head', 0);
  int(m, 'oldest', 0);
  oneOf(m, 'replay', ['partial', 'full', 'gap']);
  check(isObj(m.status), 'status');
  status(m.status);
}

function errorMessage(m) {
  str(m, 'code', { min: 1 });
  str(m, 'message');
}

const EVENT_DATA = {
  status,
  'meeting.started': (d) => {
    str(d, 'title');
    int(d, 'startedAt', 0);
    tracks(d);
    oneOf(d, 'screen', SCREENS);
    oneOf(d, 'startedBy', ['voice', 'rukoo']);
    check(has(d, 'context') && (d.context === null || isObj(d.context)), 'context');
  },
  'meeting.starting': (d) => {
    oneOf(d, 'screen', SCREENS);
    if (given(d, 'request')) str(d, 'request', { min: 1, max: 64 });
  },
  'meeting.state': (d) => {
    const state = oneOf(d, 'state', STATES);
    int(d, 'at', 0);
    if (ENDED.has(state) || given(d, 'endedAt')) int(d, 'endedAt', 0);
    if (WITH_REASON.has(state)) str(d, 'reason', { min: 1 });
    else if (given(d, 'reason')) str(d, 'reason');
  },
  speaker: (d) => {
    str(d, 'id', { min: 1 });
    oneOf(d, 'track', TRACKS);
    str(d, 'label');
  },
  passage: (d) => {
    str(d, 'id', { min: 1 });
    const track = oneOf(d, 'track', TRACKS);
    str(d, 'speaker', { nullable: true, min: 1 });
    const start = int(d, 'start', 0);
    const end = int(d, 'end', 0);
    check(end >= start, 'end');
    check(d.id === `${track}-${String(start).padStart(9, '0')}`, 'id');
    str(d, 'text');
    int(d, 'revision', 1);
    bool(d, 'dictation');
    str(d, 'language', { nullable: true });
  },
  'passage.removed': (d) => str(d, 'id', { min: 1 }),
  mark: (d) => {
    str(d, 'id', { min: 1 });
    str(d, 'kind', { min: 1 });
    const at = int(d, 'at', 0);
    if (d.until !== null) check(Number.isInteger(d.until) && d.until >= at, 'until');
    oneOf(d, 'track', MARK_TRACKS, { nullable: true });
    str(d, 'detail', { nullable: true });
    str(d, 'label', { nullable: true });
  },
  frame: (d) => {
    str(d, 'id', { min: 1 });
    int(d, 'at', 0);
    int(d, 'width', 0);
    int(d, 'height', 0);
    str(d, 'text');
  },
  'meeting.removed': (d) => { if (d.reason != null) str(d, 'reason', { min: 1, max: 64 }); }
};

function event(m) {
  int(m, 'seq', 1);
  str(m, 'event', { min: 1 });
  int(m, 'createdAt', 0);
  check(isObj(m.data), 'data');
  if (MEETING_EVENTS.has(m.event) || given(m, 'meeting')) pattern(m, 'meeting', MEETING_ID);
  const data = EVENT_DATA[m.event];
  if (data) data(m.data);
}

function reply(m) {
  str(m, 'id', { min: 1, max: 64 });
  bool(m, 'ok');
  if (m.ok) check(isObj(m.result), 'result');
  else {
    check(isObj(m.error), 'error');
    check(typeof m.error.code === 'string' && m.error.code !== '' && typeof m.error.message === 'string', 'error');
    check(!given(m.error, 'meeting') || (typeof m.error.meeting === 'string' && MEETING_ID.test(m.error.meeting)), 'error');
  }
}

const FROM = {
  rukoo: { hello, command },
  voice: { welcome, error: errorMessage, event, reply }
};

// from: who sent it, 'rukoo' or 'voice'. {ok: true, message} with a command's args defaulted to {}, or
// {ok: false, reason}.
function validate(from, message) {
  try {
    check(isObj(message), 'type');
    const kinds = FROM[from];
    const kind = kinds && typeof message.type === 'string' && Object.prototype.hasOwnProperty.call(kinds, message.type) ? kinds[message.type] : null;
    check(kind, 'type');
    return { ok: true, message: kind(message) || message };
  } catch (err) {
    if (err instanceof Invalid) return { ok: false, reason: err.reason };
    throw err;
  }
}

// The discovery file, parsed.
function validateDiscovery(file) {
  try {
    check(isObj(file), 'file');
    const protocol = int(file, 'protocol', 1);
    const min = int(file, 'minProtocol', 1);
    check(min <= protocol, 'minProtocol');
    int(file, 'port', 1);
    check(file.port <= 65535, 'port');
    pattern(file, 'token', TOKEN);
    int(file, 'pid', 0);
    pattern(file, 'instance', INSTANCE);
    str(file, 'voiceVersion');
    int(file, 'startedAt', 0);
    return { ok: true, discovery: file };
  } catch (err) {
    if (err instanceof Invalid) return { ok: false, reason: err.reason };
    throw err;
  }
}

// The highest version both ranges hold, or null. Ranges are [min, max].
function negotiate(rukoo, voice) {
  const top = Math.min(rukoo[1], voice[1]);
  return top >= Math.max(rukoo[0], voice[0]) ? top : null;
}

// Which events Voice sends after welcome ("Connecting", the replay table). Voice's own logic; Rukoo's tests and the
// fake Voice use it.
function replayPlan(helloInstance, after, instance, head, oldest) {
  if (helloInstance !== instance || after > head) return { replay: 'full', from: oldest };
  if (after < oldest - 1) return { replay: 'gap', from: oldest };
  return { replay: 'partial', from: after + 1 };
}

const isMeetingId = (v) => typeof v === 'string' && MEETING_ID.test(v);

module.exports = {
  PROTOCOL,
  MIN_PROTOCOL,
  VOICE_MESSAGE_MAX,
  RUKOO_MESSAGE_MAX,
  DISCOVERY_MAX,
  TITLE_MAX,
  LABEL_MAX,
  CONTEXT_MAX,
  TRACKS,
  SCREENS,
  STATES,
  ERROR_CODES,
  MEETING_EVENTS,
  MEDIA_KINDS,
  MEDIA_ID,
  MEDIA_SPAN_MAX,
  MEDIA_READ_MAX,
  validate,
  validateDiscovery,
  negotiate,
  replayPlan,
  isMeetingId
};
