'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const { PERMISSIONS } = require('./permission');

// The built-in agents. Custom agents (the `custom` list) come on top; agentIds() lists both.
const AGENT_IDS = ['clark', 'claude', 'codex', 'openclaw'];
// Agents that keep a secret (the Hermes API key, the OpenClaw gateway token), encrypted in the file as `key`.
// Every custom agent keeps one too.
const SECRET_AGENTS = ['clark', 'openclaw'];

// Custom agents: the user's own agents on a server that speaks a protocol Rukoo knows, such as Booper or another
// agent framework. Each has an id of its own (no ':', which Settings actions split on), a name, a server and a key.
// protocol 'runs': the Hermes Runs API (hermes.js), which Hermes and Booper serve. protocol 'openai': the
// OpenAI-compatible Chat Completions endpoint most agent frameworks serve (openai.js). preset only picks the
// defaults, the wording of the setup Rukoo copies and the card's help; the protocol decides how Rukoo talks to it.
const CUSTOM_ID = /^cx_[a-z0-9]{10}$/;
const CUSTOM_MAX = 8;
const CUSTOM_PROTOCOLS = ['runs', 'openai'];
const CUSTOM_PRESETS = {
  booper: { protocol: 'runs', name: 'Booper', url: 'http://127.0.0.1:8662' },
  runs: { protocol: 'runs', name: 'Runs API agent', url: '' },
  openai: { protocol: 'openai', name: 'OpenAI-compatible agent', url: 'http://127.0.0.1:8000' }
};
// A custom agent as stored. key: the server's API key or token, encrypted ('' or null when the server takes none).
// seed: a random secret, encrypted, that Rukoo makes its MCP tokens for this agent from (customTokenFor), so the
// agent gets Rukoo's tools even without a key and a new key doesn't break its setup. remote: the user lets the agent
// use Rukoo's tools from another computer, which opens the Tailscale listener. history (openai only): Rukoo sends the
// chat's earlier turns with each message, for a server that keeps no conversation itself.
const CUSTOM_DEFAULTS = { id: '', protocol: 'runs', preset: 'runs', name: '', enabled: true, url: '', key: null, seed: null, model: '', remote: false, history: true };
// Where an OpenClaw gateway listens unless it was told otherwise (docs.openclaw.ai/gateway/config-gateway).
const OPENCLAW_URL = 'http://127.0.0.1:18789';
const ACCESS = ['ask', 'full'];
// Every effort level an agent may offer, lowest first. Each agent offers its own subset (see models()).
const EFFORTS = ['minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];

// The chat panel's shortcuts, as the user arranged them (null: Rukoo's own, in their own order). Each entry is a
// command name with what the user changed: a shortcut of Rukoo's can be hidden or get its own label and prompt; one
// of the user's own (custom) has all of these. Main checks the shape; the panel knows which names are Rukoo's.
const SHORTCUT_KEY = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
// Message actions the hub and the panel give a meaning of their own; a shortcut can't take their name.
const SHORTCUT_RESERVED = new Set(['approved', 'declined', 'skill', 'shortcut']);
const SHORTCUT_ICONS = ['sparkle', 'mail', 'search', 'list', 'arrow-right', 'layers', 'clock', 'file', 'pen', 'calendar', 'person', 'globe', 'chart', 'link', 'check', 'shield', 'info', 'x'];
const SHORTCUT_WHEN = ['email', 'always'];
const SHORTCUTS_MAX = 40;
const SHORTCUT_LABEL_MAX = 60;
const SHORTCUT_PROMPT_MAX = 4000;

const DEFAULTS = {
  version: 1,
  defaultAgent: 'clark',
  // The permission level a chat has unless it picked its own (permission.js). Replaces autoMailActions, the switch
  // that let mail actions other than deleting and unsubscribing run without a card: load() never turns that into
  // Auto, which would also add events without asking.
  permission: 'ask',
  shortcuts: null,
  clark: { enabled: true, name: 'Hermes', url: '', key: null, model: '' },
  claude: { enabled: true, exe: '', configDir: '', model: '', access: 'ask' },
  codex: { enabled: true, exe: '', model: '', access: 'ask' },
  // agent: the OpenClaw agent id a new chat goes to ('' for the gateway's default agent). model: a backend model
  // for x-openclaw-model ('' for the agent's own). remote: the user lets a gateway on another computer use Rukoo's
  // tools over Tailscale, which opens the Tailscale listener (see hub.wantsRemote()).
  openclaw: { enabled: true, url: OPENCLAW_URL, key: null, agent: '', model: '', remote: false },
  mcp: { port: 47800, remote: false, remoteHost: '' },
  // enabled: "Let my phone use this computer", which opens the Tailscale listener for the phone routes
  // (phonehost.js). pairings: the phones paired with a code, see loadPairings().
  phone: { enabled: false, pairings: [] }
};

// A phone's pairing as stored. id has no ':' because Settings actions split on it. tokenHash is the SHA-256 (hex) of
// the token the phone got at pairing: the desktop only ever compares tokens, so it never needs the token back.
const PAIRING_ID = /^ph_[a-z0-9]{10}$/;
const TOKEN_HASH = /^[0-9a-f]{64}$/;
const MAX_PAIRINGS = 20;
// lastSeenAt is written at most this often, so a phone polling every few seconds doesn't rewrite agents.json.
const SEEN_SAVE_MS = 60 * 1000;

const oneLine = (v, max) => String(v == null ? '' : v).replace(/\s+/g, ' ').trim().slice(0, max);
const time = (v) => (Number.isFinite(v) && v > 0 ? v : null);

// Pairings from the file: mergeSection would copy the array without looking at its entries.
function loadPairings(list) {
  const out = [];
  for (const p of Array.isArray(list) ? list : []) {
    if (out.length >= MAX_PAIRINGS) break;
    if (!isObj(p) || !PAIRING_ID.test(p.id) || typeof p.name !== 'string' || !TOKEN_HASH.test(p.tokenHash)) continue;
    if (out.some((x) => x.id === p.id)) continue;
    out.push({
      id: p.id,
      name: oneLine(p.name, 60) || 'Phone',
      platform: oneLine(p.platform, 20) || 'android',
      model: oneLine(p.model, 60),
      createdAt: time(p.createdAt) || Date.now(),
      lastSeenAt: time(p.lastSeenAt),
      lastExportAt: time(p.lastExportAt),
      exportUntil: time(p.exportUntil),
      tokenHash: p.tokenHash
    });
  }
  return out;
}

// Clark's bridge signs in with a token made from the Hermes API key. Every Rukoo needs that key to chat with
// Clark anyway, so one bridge setup reaches Rukoo on any of the user's devices.
function remoteTokenFor(key) {
  return key ? crypto.createHmac('sha256', key).update('rukoo-mcp-v1').digest('base64url') : '';
}

// The bridge as a single file from a release tag, so it matches the Rukoo it comes with.
const BRIDGE_DOWNLOAD = 'https://raw.githubusercontent.com/MajesteitBart/rukoo/<tag>/integrations/hermes/rukoo_bridge.py';

// The tokens OpenClaw's MCP client sends to Rukoo, made from the gateway token: they change with that token, and
// they don't give away the gateway token itself. There are two, so each works on one listener only: the local one on
// 127.0.0.1, the remote one on the Tailscale listener, and that one only while the user allows a remote gateway.
// A local token that leaks from this computer is then of no use from the tailnet.
function openclawTokenFor(key, remote = false) {
  return key ? crypto.createHmac('sha256', key).update(remote ? 'rukoo-mcp-openclaw-remote-v1' : 'rukoo-mcp-openclaw-v1').digest('base64url') : '';
}

// The variable OpenClaw's MCP entry for Rukoo reads its token from. One name for the local and the remote token, and
// for the phone's: OpenClaw holds one Rukoo entry at a time.
const OPENCLAW_TOKEN_VAR = 'RUKOO_MCP_TOKEN';

// The tokens a custom agent's MCP client sends to Rukoo, made from its seed: one for the 127.0.0.1 listener and one for
// the Tailscale listener, like OpenClaw's. The agent id goes in too, so two agents never share a token.
function customTokenFor(seed, id, remote = false) {
  return seed && id ? crypto.createHmac('sha256', seed).update(`${remote ? 'rukoo-mcp-custom-remote-v1' : 'rukoo-mcp-custom-v1'}:${id}`).digest('base64url') : '';
}

// The tools of Rukoo's that only read: an agent that reruns an interrupted call of one of these after a restart
// changes nothing (Booper's safeTools).
const READ_ONLY_TOOLS = ['get_context', 'search_mail', 'read_message', 'read_attachment', 'read_chat_file', 'get_draft', 'read_skill', 'list_events', 'find_free_time'];

// Custom agents from the file: mergeSection would copy the array without looking at its entries.
function loadCustom(list) {
  const out = [];
  for (const c of Array.isArray(list) ? list : []) {
    if (out.length >= CUSTOM_MAX) break;
    if (!isObj(c) || !CUSTOM_ID.test(c.id) || out.some((x) => x.id === c.id)) continue;
    const entry = mergeSection(CUSTOM_DEFAULTS, c);
    if (!CUSTOM_PROTOCOLS.includes(entry.protocol)) continue;
    if (!CUSTOM_PRESETS[entry.preset] || CUSTOM_PRESETS[entry.preset].protocol !== entry.protocol) entry.preset = entry.protocol;
    entry.name = oneLine(entry.name, 40) || CUSTOM_PRESETS[entry.preset].name;
    out.push(entry);
  }
  return out;
}

// An OpenClaw agent id as the gateway takes it in `openclaw/<agentId>` (lowercase letters, digits, - and _).
const OPENCLAW_AGENT = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/;

// Errors that reach the renderer carry a code from SPEC 4.6; the message is the bare code so the
// renderer can localize it from Error.message after the IPC round trip.
class AgentError extends Error {
  constructor(code, detail = '') {
    super(code);
    this.code = code;
    this.detail = String(detail || '');
  }
}

function writeJsonAtomic(file, data) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(data));
  fs.renameSync(tmp, file);
}

const clone = (v) => JSON.parse(JSON.stringify(v));
const isObj = (v) => Boolean(v) && typeof v === 'object' && !Array.isArray(v);

// Tailscale hands out addresses from the CGNAT range 100.64.0.0/10.
function isTailscale(ip) {
  const m = /^100\.(\d+)\.\d+\.\d+$/.exec(String(ip || ''));
  return Boolean(m) && Number(m[1]) >= 64 && Number(m[1]) <= 127;
}

function tailscaleAddress(interfaces = os.networkInterfaces) {
  let list = {};
  try {
    list = interfaces() || {};
  } catch (_) {
    return '';
  }
  for (const addrs of Object.values(list)) {
    for (const a of addrs || []) {
      const v4 = a.family === 'IPv4' || a.family === 4;
      if (v4 && !a.internal && isTailscale(a.address)) return a.address;
    }
  }
  return '';
}

// Copies the known keys of a stored section over the defaults, keeping only values of the right type.
function mergeSection(defaults, stored) {
  const out = { ...defaults };
  if (!isObj(stored)) return out;
  for (const [k, def] of Object.entries(defaults)) {
    const v = stored[k];
    if (v === undefined) continue;
    if (def === null ? typeof v === 'string' || v === null : typeof v === typeof def) out[k] = v;
  }
  return out;
}

function text(value, max, field) {
  if (typeof value !== 'string') throw new AgentError('invalid', `${field} must be text`);
  const s = value.trim();
  if (s.length > max) throw new AgentError('invalid', `${field} is longer than ${max} characters`);
  return s;
}

function bool(value, field) {
  if (typeof value !== 'boolean') throw new AgentError('invalid', `${field} must be true or false`);
  return value;
}

// The shortcut list as stored: a clean copy, in order. An entry that breaks the rules throws 'invalid'.
function shortcutList(value) {
  if (value === null) return null;
  if (!Array.isArray(value)) throw new AgentError('invalid', 'shortcuts must be a list');
  if (value.length > SHORTCUTS_MAX) throw new AgentError('invalid', `at most ${SHORTCUTS_MAX} shortcuts`);
  const seen = new Set();
  return value.map((entry) => {
    if (!isObj(entry)) throw new AgentError('invalid', 'a shortcut must be an object');
    const key = text(entry.key, 40, 'shortcut name').toLowerCase();
    if (!SHORTCUT_KEY.test(key)) throw new AgentError('invalid', 'a shortcut name has lowercase letters, digits and single hyphens');
    if (SHORTCUT_RESERVED.has(key)) throw new AgentError('invalid', `/${key} is taken`);
    if (seen.has(key)) throw new AgentError('invalid', `/${key} is there twice`);
    seen.add(key);
    const out = { key };
    if (entry.custom === true) out.custom = true;
    if (entry.hidden !== undefined && bool(entry.hidden, 'shortcut hidden')) out.hidden = true;
    const label = entry.label == null ? '' : text(entry.label, SHORTCUT_LABEL_MAX, 'shortcut label');
    const prompt = entry.prompt == null ? '' : text(entry.prompt, SHORTCUT_PROMPT_MAX, 'shortcut prompt');
    if (label) out.label = label;
    if (prompt) out.prompt = prompt;
    if (out.custom) {
      if (!label || !prompt) throw new AgentError('invalid', `/${key} needs a label and a prompt`);
      out.icon = SHORTCUT_ICONS.includes(entry.icon) ? entry.icon : 'sparkle';
      out.when = SHORTCUT_WHEN.includes(entry.when) ? entry.when : 'email';
    }
    return out;
  });
}

// A list in the file that no longer passes (edited by hand, or from a later Rukoo) gives way to the defaults.
function storedShortcuts(value) {
  try {
    return value === undefined ? null : shortcutList(value);
  } catch (_) {
    return null;
  }
}

function httpUrl(value, field) {
  const s = text(value, 300, field);
  if (!s) return '';
  let u;
  try {
    u = new URL(s);
  } catch (_) {
    throw new AgentError('invalid', `${field} is not a valid URL`);
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') throw new AgentError('invalid', `${field} must start with http:// or https://`);
  if (u.username || u.password) throw new AgentError('invalid', `${field} must not contain credentials`);
  return s.replace(/\/+$/, '');
}

class AgentConfig {
  constructor({ file, secrets, interfaces = os.networkInterfaces } = {}) {
    this.file = file;
    this.secrets = secrets || { encrypt: (s) => s, decrypt: (s) => s };
    this.interfaces = interfaces;
    this.data = clone(DEFAULTS);
    // Filled in by the hub once the MCP listeners are up; never persisted.
    this.runtime = { localPort: null, remotePort: null, note: '' };
    // Per pairing, when its lastSeenAt was last written to the file (touchPairing()).
    this.seenSaved = new Map();
  }

  load() {
    let stored = null;
    try {
      stored = JSON.parse(fs.readFileSync(this.file, 'utf8'));
    } catch (_) {
      stored = null;
    }
    const s = isObj(stored) ? stored : {};
    this.data = {
      version: 1,
      // Checked below, once the custom agents are in.
      defaultAgent: typeof s.defaultAgent === 'string' ? s.defaultAgent : DEFAULTS.defaultAgent,
      // A file from before the levels has autoMailActions instead, and starts at Ask.
      permission: PERMISSIONS.includes(s.permission) ? s.permission : DEFAULTS.permission,
      shortcuts: storedShortcuts(s.shortcuts),
      clark: mergeSection(DEFAULTS.clark, s.clark),
      claude: mergeSection(DEFAULTS.claude, s.claude),
      codex: mergeSection(DEFAULTS.codex, s.codex),
      openclaw: mergeSection(DEFAULTS.openclaw, s.openclaw),
      mcp: mergeSection(DEFAULTS.mcp, s.mcp),
      phone: { enabled: isObj(s.phone) && s.phone.enabled === true, pairings: loadPairings(isObj(s.phone) ? s.phone.pairings : []) },
      custom: loadCustom(s.custom)
    };
    // A file from a later Rukoo, or an agent removed by hand: the default goes back to Hermes.
    if (!this.agentIds().includes(this.data.defaultAgent)) this.data.defaultAgent = DEFAULTS.defaultAgent;
    if (this.data.openclaw.agent && !OPENCLAW_AGENT.test(this.data.openclaw.agent)) this.data.openclaw.agent = '';
    for (const id of ['claude', 'codex']) if (!ACCESS.includes(this.data[id].access)) this.data[id].access = 'ask';
    const port = this.data.mcp.port;
    if (!Number.isInteger(port) || port < 1024 || port > 65535) this.data.mcp.port = DEFAULTS.mcp.port;
    if (!this.data.clark.name) this.data.clark.name = DEFAULTS.clark.name;
    return this;
  }

  save() {
    writeJsonAtomic(this.file, this.data);
  }

  decrypt(value) {
    if (!value) return '';
    try {
      return this.secrets.decrypt(value);
    } catch (_) {
      // Encrypted under another Windows user or machine: treat as not set.
      return '';
    }
  }

  // Raw section for main-process code, with the secret decrypted. Never send this to the renderer.
  get(section) {
    if (isCustomId(section)) {
      const c = this.custom(section);
      if (!c) return null;
      const { seed, ...rest } = c;
      return { ...rest, key: this.decrypt(c.key) };
    }
    if (SECRET_AGENTS.includes(section)) return { ...this.data[section], key: this.decrypt(this.data[section].key) };
    if (this.data[section] && isObj(this.data[section])) return { ...this.data[section] };
    return null;
  }

  // ---------- agents, built in and custom ----------

  // Every agent a chat can have now: the built-in ones, then the custom ones in the order the user added them.
  agentIds() {
    return [...AGENT_IDS, ...(this.data.custom || []).map((c) => c.id)];
  }

  hasAgent(id) {
    return this.agentIds().includes(id);
  }

  custom(id) {
    return (isCustomId(id) && (this.data.custom || []).find((c) => c.id === id)) || null;
  }

  // The stored settings of an agent (its section, or its entry in the custom list); null for an agent that isn't
  // there, such as a custom agent the user removed while chats with it remain.
  agentData(id) {
    if (isCustomId(id)) return this.custom(id);
    return AGENT_IDS.includes(id) ? this.data[id] : null;
  }

  // Whether the agent is there and turned on. A removed custom agent counts as off: its chats stay readable.
  enabled(id) {
    const d = this.agentData(id);
    return Boolean(d) && d.enabled !== false;
  }

  displayName(agent) {
    if (agent === 'clark') return this.data.clark.name || DEFAULTS.clark.name;
    if (isCustomId(agent)) {
      const c = this.custom(agent);
      return c ? c.name : 'Removed agent';
    }
    return agent === 'claude' ? 'Claude' : agent === 'codex' ? 'Codex' : agent === 'openclaw' ? 'OpenClaw' : String(agent || '');
  }

  // Adds a custom agent with the defaults of a preset (CUSTOM_PRESETS) and returns its id.
  addCustom({ preset = 'runs', random = crypto.randomBytes } = {}) {
    const p = CUSTOM_PRESETS[preset];
    if (!p) throw new AgentError('invalid', 'preset must be booper, runs or openai');
    if (this.data.custom.length >= CUSTOM_MAX) throw new AgentError('invalid', `at most ${CUSTOM_MAX} custom agents`);
    let id = '';
    do {
      id = `cx_${[...random(10)].map((b) => (b % 36).toString(36)).join('')}`;
    } while (this.data.custom.some((c) => c.id === id));
    const taken = new Set(this.agentIds().map((a) => this.displayName(a).toLowerCase()));
    let name = p.name;
    for (let n = 2; taken.has(name.toLowerCase()); n++) name = `${p.name} ${n}`;
    // No seed yet: it is made when the user first copies the setup (ensureSeed), so adding an agent works without the
    // system keyring, which Linux needs for every secret.
    this.data.custom.push({ ...CUSTOM_DEFAULTS, id, preset, protocol: p.protocol, name, url: p.url });
    this.save();
    return id;
  }

  // The seed the agent's MCP tokens come from, made once. Throws when the system can't keep a secret (Linux without a
  // keyring), as setSecret does.
  ensureSeed(id, random = crypto.randomBytes) {
    const c = this.custom(id);
    if (!c) throw new AgentError('invalid', 'unknown custom agent');
    if (c.seed && this.decrypt(c.seed)) return;
    c.seed = this.secrets.encrypt(random(32).toString('base64url'));
    this.save();
  }

  removeCustom(id) {
    const before = this.data.custom.length;
    this.data.custom = this.data.custom.filter((c) => c.id !== id);
    if (this.data.custom.length === before) return false;
    if (this.data.defaultAgent === id) this.data.defaultAgent = DEFAULTS.defaultAgent;
    this.save();
    return true;
  }

  // remoteHost wins; otherwise the first Tailscale IPv4 on this machine.
  address() {
    return this.data.mcp.remoteHost || tailscaleAddress(this.interfaces);
  }

  publicView() {
    const d = this.data;
    const { key, ...clark } = d.clark;
    const { key: openclawKey, ...openclaw } = d.openclaw;
    return {
      version: 1,
      defaultAgent: d.defaultAgent,
      permission: d.permission,
      shortcuts: d.shortcuts ? clone(d.shortcuts) : null,
      clark: { ...clark, hasKey: Boolean(key) },
      claude: { ...d.claude },
      codex: { ...d.codex },
      openclaw: { ...openclaw, hasKey: Boolean(openclawKey) },
      // Never the key or the seed.
      custom: (d.custom || []).map(({ key: customKey, seed, ...c }) => ({ ...c, hasKey: Boolean(customKey) })),
      mcp: {
        port: d.mcp.port,
        remote: d.mcp.remote,
        remoteHost: d.mcp.remoteHost,
        address: this.address(),
        localPort: this.runtime.localPort,
        remotePort: this.runtime.remotePort,
        note: this.runtime.note || ''
      },
      // Never the token hash.
      phone: {
        enabled: d.phone.enabled,
        pairings: d.phone.pairings.map((p) => ({
          id: p.id,
          name: p.name,
          platform: p.platform,
          model: p.model,
          createdAt: p.createdAt,
          lastSeenAt: p.lastSeenAt,
          lastExportAt: p.lastExportAt,
          exportOpen: Boolean(p.exportUntil && p.exportUntil > Date.now())
        }))
      }
    };
  }

  // Applies a renderer patch. Unknown keys are ignored; known keys with bad values throw 'invalid'.
  // Secrets never come in this way (setSecret).
  update(patch) {
    if (!isObj(patch)) throw new AgentError('invalid', 'patch must be an object');
    const next = clone(this.data);
    if ('defaultAgent' in patch) {
      if (!this.agentIds().includes(patch.defaultAgent)) throw new AgentError('invalid', 'defaultAgent must be clark, claude, codex, openclaw or a custom agent');
      next.defaultAgent = patch.defaultAgent;
    }
    if ('permission' in patch) {
      if (!PERMISSIONS.includes(patch.permission)) throw new AgentError('invalid', 'permission must be ask, auto or full');
      next.permission = patch.permission;
    }
    if ('shortcuts' in patch) next.shortcuts = shortcutList(patch.shortcuts);
    const section = (name, apply) => {
      if (!(name in patch)) return;
      if (!isObj(patch[name])) throw new AgentError('invalid', `${name} must be an object`);
      apply(patch[name], next[name]);
    };
    const access = (v, field) => {
      if (!ACCESS.includes(v)) throw new AgentError('invalid', `${field} must be ask or full`);
      return v;
    };
    section('clark', (p, s) => {
      if ('enabled' in p) s.enabled = bool(p.enabled, 'clark.enabled');
      if ('name' in p) s.name = text(p.name, 40, 'clark.name') || DEFAULTS.clark.name;
      if ('url' in p) s.url = httpUrl(p.url, 'clark.url');
      if ('model' in p) s.model = text(p.model, 100, 'clark.model');
    });
    section('claude', (p, s) => {
      if ('enabled' in p) s.enabled = bool(p.enabled, 'claude.enabled');
      if ('exe' in p) s.exe = text(p.exe, 500, 'claude.exe');
      if ('configDir' in p) s.configDir = text(p.configDir, 500, 'claude.configDir');
      if ('model' in p) s.model = text(p.model, 100, 'claude.model');
      if ('access' in p) s.access = access(p.access, 'claude.access');
    });
    section('codex', (p, s) => {
      if ('enabled' in p) s.enabled = bool(p.enabled, 'codex.enabled');
      if ('exe' in p) s.exe = text(p.exe, 500, 'codex.exe');
      if ('model' in p) s.model = text(p.model, 100, 'codex.model');
      if ('access' in p) s.access = access(p.access, 'codex.access');
    });
    section('openclaw', (p, s) => {
      if ('enabled' in p) s.enabled = bool(p.enabled, 'openclaw.enabled');
      if ('url' in p) s.url = httpUrl(p.url, 'openclaw.url');
      if ('agent' in p) {
        const agent = text(p.agent, 64, 'openclaw.agent');
        if (agent && !OPENCLAW_AGENT.test(agent)) throw new AgentError('invalid', 'openclaw.agent is an agent id: letters, digits, - and _');
        s.agent = agent;
      }
      if ('model' in p) s.model = text(p.model, 100, 'openclaw.model');
      if ('remote' in p) s.remote = bool(p.remote, 'openclaw.remote');
    });
    section('mcp', (p, s) => {
      if ('port' in p) {
        const port = Number(p.port);
        if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new AgentError('invalid', 'mcp.port must be between 1024 and 65535');
        s.port = port;
      }
      if ('remote' in p) s.remote = bool(p.remote, 'mcp.remote');
      if ('remoteHost' in p) {
        const host = text(p.remoteHost, 255, 'mcp.remoteHost');
        if (host && !/^[A-Za-z0-9.\-]+$/.test(host)) throw new AgentError('invalid', 'mcp.remoteHost must be an IPv4 address or host name');
        s.remoteHost = host;
      }
    });
    // {custom: {<id>: {...}}}: changes to custom agents that are there. Adding and removing have their own methods.
    if ('custom' in patch) {
      if (!isObj(patch.custom)) throw new AgentError('invalid', 'custom must be an object');
      for (const [id, p] of Object.entries(patch.custom)) {
        const s = (next.custom || []).find((c) => c.id === id);
        if (!s) throw new AgentError('invalid', 'unknown custom agent');
        if (!isObj(p)) throw new AgentError('invalid', 'a custom agent must be an object');
        if ('enabled' in p) s.enabled = bool(p.enabled, 'custom.enabled');
        if ('name' in p) s.name = oneLine(text(p.name, 40, 'custom.name'), 40) || CUSTOM_PRESETS[s.preset].name;
        if ('url' in p) s.url = httpUrl(p.url, 'custom.url');
        if ('model' in p) s.model = text(p.model, 100, 'custom.model');
        if ('remote' in p) s.remote = bool(p.remote, 'custom.remote');
        if ('history' in p) s.history = bool(p.history, 'custom.history');
      }
    }
    // Pairings change only through the phone routes and their own methods below, never from a renderer patch.
    section('phone', (p, s) => {
      if ('enabled' in p) s.enabled = bool(p.enabled, 'phone.enabled');
    });
    this.data = next;
    this.save();
    return this.publicView();
  }

  // ---------- phone pairings (phonehost.js) ----------

  pairing(id) {
    return this.data.phone.pairings.find((p) => p.id === id) || null;
  }

  // tokenHash: the SHA-256 (hex) of the phone's token. Returns the stored pairing, or null when the list is full.
  addPairing({ id, name, platform, model, tokenHash, exportUntil = null, now = Date.now() }) {
    if (this.data.phone.pairings.length >= MAX_PAIRINGS) return null;
    const p = {
      id,
      name: oneLine(name, 60) || 'Phone',
      platform: oneLine(platform, 20) || 'android',
      model: oneLine(model, 60),
      createdAt: now,
      lastSeenAt: now,
      lastExportAt: null,
      exportUntil: time(exportUntil),
      tokenHash
    };
    this.data.phone.pairings.push(p);
    this.seenSaved.set(id, now);
    this.save();
    return p;
  }

  removePairing(id) {
    const before = this.data.phone.pairings.length;
    this.data.phone.pairings = this.data.phone.pairings.filter((p) => p.id !== id);
    if (this.data.phone.pairings.length === before) return false;
    this.save();
    return true;
  }

  // Returns whether it saved, so the hub can tell Settings that "Last seen" moved. Counted from the last save, so a
  // phone that asks every few seconds still gets saved once a minute.
  touchPairing(id, at = Date.now()) {
    const p = this.pairing(id);
    if (!p) return false;
    p.lastSeenAt = at;
    const saved = this.seenSaved.get(id);
    if (saved !== undefined && at - saved < SEEN_SAVE_MS) return false;
    this.seenSaved.set(id, at);
    this.save();
    return true;
  }

  // until: when the phone may no longer fetch the export, or null to close it. exported: it was just sent.
  setExportWindow(id, until, { exported = false, now = Date.now() } = {}) {
    const p = this.pairing(id);
    if (!p) return null;
    p.exportUntil = time(until);
    if (exported) p.lastExportAt = now;
    this.save();
    return p;
  }

  pairingHashes() {
    return this.data.phone.pairings.map((p) => ({ id: p.id, hash: Buffer.from(p.tokenHash, 'hex') }));
  }

  setSecret(agent, value) {
    const section = SECRET_AGENTS.includes(agent) ? this.data[agent] : this.custom(agent);
    if (!section) throw new AgentError('invalid', 'only clark, openclaw and custom agents have a secret');
    if (typeof value !== 'string' || value.length > 4000) throw new AgentError('invalid', 'key must be text');
    const v = value.trim();
    section.key = v ? this.secrets.encrypt(v) : null;
    this.save();
    return this.publicView();
  }

  // The token Clark's bridge sends; empty until the API key is set.
  remoteToken() {
    return remoteTokenFor(this.get('clark').key);
  }

  // A token OpenClaw's MCP client sends (see openclawTokenFor); empty until the gateway token is set. The remote one
  // also stays empty while the user doesn't allow a gateway on another computer, so the Tailscale listener, which
  // may be open for Hermes, takes no OpenClaw token then.
  openclawToken(remote = false) {
    if (remote && !this.data.openclaw.remote) return '';
    return openclawTokenFor(this.get('openclaw').key, remote);
  }

  // A token a custom agent's MCP client sends (customTokenFor); empty until the agent has a seed (ensureSeed). The
  // remote one stays empty while the user doesn't let that agent use Rukoo from another computer, or the agent is off.
  customToken(id, remote = false) {
    const c = this.custom(id);
    if (!c || (remote && !(c.remote && c.enabled !== false))) return '';
    return customTokenFor(this.decrypt(c.seed), id, remote);
  }

  // What the user pastes on the agent's machine so it gets Rukoo's tools: for Booper the block for booper.yaml and
  // the environment variable its header comes from, for any other agent the server as most MCP clients take it.
  // Holds the token, like the OpenClaw command: the agent can't compute it.
  customSetup(id, { remote = false, host = '127.0.0.1', port = this.runtime.localPort || this.data.mcp.port } = {}) {
    const c = this.custom(id);
    const token = this.customToken(id, remote);
    if (!c || !token || !host || !port) return '';
    const url = `http://${host}:${port}/mcp`;
    const where = remote ? 'from another computer in your tailnet' : 'on this computer';
    if (c.preset === 'booper') {
      return [
        `# Rukoo Mail: lets ${c.name} read mail, write drafts and ask for approval in Rukoo, ${where}.`,
        '# 1. Give Booper this environment variable (in the environment its service starts with), replacing an older one:',
        `#      RUKOO_MCP_AUTHORIZATION=Bearer ${token}`,
        '# 2. Merge this block into booper.yaml (keep your other mcp servers) and restart Booper.',
        'mcp:',
        '  servers:',
        '    rukoo:',
        `      url: ${url}`,
        '      headersEnv: { Authorization: RUKOO_MCP_AUTHORIZATION }',
        '      tools: ["*"]',
        `      safeTools: [${READ_ONLY_TOOLS.join(', ')}]`,
        ''
      ].join('\n');
    }
    return [
      `// Rukoo Mail: lets ${c.name} read mail, write drafts and ask for approval in Rukoo, ${where}.`,
      '// Add this MCP server (Streamable HTTP) to the agent and restart it. The token is a secret.',
      JSON.stringify({ rukoo: { url, transport: 'streamable-http', headers: { Authorization: `Bearer ${token}` } } }, null, 2),
      ''
    ].join('\n');
  }

  // The two steps that add Rukoo to OpenClaw's MCP servers (docs.openclaw.ai/cli/mcp), as Streamable HTTP; OpenClaw
  // leaves a URL server on SSE unless transport says otherwise. Step 1 is a line for the gateway's .env file with the
  // token for that listener. Step 2 is the command, whose header names the variable, so the token stays out of
  // openclaw.json, where `openclaw mcp doctor` flags a literal one. OpenClaw fills the variable in from its own
  // environment or that file each time it connects. The comment lines hold no quotes, brackets or other shell syntax,
  // so pasting the whole text into a shell does no harm, and the single quotes keep ${RUKOO_MCP_TOKEN} from the shell.
  // Android's McpSetup.openClaw copies the same text.
  // {remote: false, port}: this computer's loopback, the port Rukoo's local listener really got.
  // {remote: true, host, port}: Rukoo's Tailscale address and the port its Tailscale listener really got.
  // The text holds a token, unlike the Hermes block: OpenClaw can't compute it from its gateway token.
  openclawSetup({ remote = false, host = '127.0.0.1', port = this.runtime.localPort || this.data.mcp.port } = {}) {
    const token = this.openclawToken(remote);
    if (!token || !host || !port) return '';
    const server = { url: `http://${host}:${port}/mcp`, transport: 'streamable-http', headers: { Authorization: `Bearer \${${OPENCLAW_TOKEN_VAR}}` } };
    return [
      `# Step 1, not a command: put this line in the OpenClaw .env file, ~/.openclaw/.env or $OPENCLAW_STATE_DIR/.env, in place of any older ${OPENCLAW_TOKEN_VAR} line.`,
      `${OPENCLAW_TOKEN_VAR}=${token}`,
      '# Step 2: run this command where the gateway runs, then restart the gateway.',
      `openclaw mcp set rukoo '${JSON.stringify(server)}'`,
      ''
    ].join('\n');
  }

  // The block for ~/.hermes/config.yaml on Clark's machine. It holds no secret and is the same on every device:
  // the bridge finds Rukoo over Tailscale and signs in with the API server key, which Hermes fills in from its
  // .env. ${userHome} is a Hermes config variable. version is this Rukoo's: the bridge comes from the same release
  // (tagged v<version>), so the two speak the same bridge protocol and nobody needs a checkout of the repository.
  hermesSetup({ version = '' } = {}) {
    // The configured port, which other devices share. A port this device fell back to applies to it alone.
    const port = this.data.mcp.port;
    const name = this.displayName('clark');
    const release = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version) ? `v${version}` : '';
    return [
      `# Rukoo Mail: lets ${name} read mail, write drafts and ask for approval in Rukoo, on any of your devices.`,
      ...(release
        ? [
            `# 1. Download the bridge for Rukoo ${version} to ~/.hermes/ on this machine:`,
            `#    curl -fsSL -o ~/.hermes/rukoo_bridge.py ${BRIDGE_DOWNLOAD.replace('<tag>', release)}`
          ]
        : ['# 1. Copy rukoo_bridge.py (Rukoo: integrations/hermes) to ~/.hermes/ on this machine.']),
      '# 2. Merge this block into ~/.hermes/config.yaml (keep your other mcp_servers).',
      '# 3. Check it with: hermes mcp test rukoo',
      `# On each device, turn on Settings > Agents > "Let ${name} use Rukoo".`,
      'mcp_servers:',
      '  rukoo:',
      '    command: python3',
      '    args: ["${userHome}/.hermes/rukoo_bridge.py"]',
      '    env:',
      '      RUKOO_KEY: ${API_SERVER_KEY}',
      ...(port !== DEFAULTS.mcp.port ? [`      RUKOO_PORT: "${port}"`] : []),
      '    timeout: 120',
      ''
    ].join('\n');
  }
}

function isCustomId(id) {
  return typeof id === 'string' && CUSTOM_ID.test(id);
}

module.exports = {
  AgentConfig,
  AgentError,
  AGENT_IDS,
  CUSTOM_MAX,
  CUSTOM_PRESETS,
  READ_ONLY_TOOLS,
  isCustomId,
  customTokenFor,
  SECRET_AGENTS,
  EFFORTS,
  DEFAULTS,
  SHORTCUT_ICONS,
  SHORTCUTS_MAX,
  MAX_PAIRINGS,
  writeJsonAtomic,
  tailscaleAddress,
  isTailscale,
  remoteTokenFor,
  openclawTokenFor
};
