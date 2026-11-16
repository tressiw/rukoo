'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const { EventEmitter } = require('events');
const { AgentConfig, AgentError, AGENT_IDS, EFFORTS, writeJsonAtomic, tailscaleAddress, isCustomId } = require('./config');
const { McpServer, INSTRUCTIONS: MCP_INSTRUCTIONS, json: mcpJson } = require('./mcp');
const { decodeId } = require('../engine');
const tools = require('./tools');
const context = require('./context');
const skills = require('./skills');
const ownskills = require('./ownskills');
const { ChatFiles, shown: shownFile, MAX_FILES, MAX_EMAILS } = require('./chatfiles');
const { PdfReader } = require('../pdfread');
const { PhoneHost } = require('./phonehost');
const { MediaLinks } = require('./media');
const { isMeetingId } = require('../voice/protocol');
const { PERMISSIONS, permissionLevel, strictest } = require('./permission');
// Agents whose tool calls are bound to their chat (resolveCall()): Claude Code by a token for each chat, Codex by its
// thread id. agent/panel.js has the same list.
const BOUND_AGENTS = new Set(['claude', 'codex']);

const MAX_CONVERSATIONS = 150;
// Conversations an agent opened from outside the panel (Clark on WhatsApp) have their own, smaller cap.
const MAX_EXTERNAL = 20;
// An agent calling in from outside keeps using one conversation per open email for this long.
const EXTERNAL_REUSE_MS = 4 * 60 * 60 * 1000;
const MAX_ITEMS = 400;
const MAX_TEXT = 40000;
// Newer emails in its thread a chat keeps after the user continued it from them, besides those the agent has not had
// yet; see continueFrom(). At most MAX_NEWER of them go along with one message (loadNewer()).
const MAX_CONTINUED = 20;
const MAX_NEWER = 3;
// Messages that can wait in one chat while the agent works (see hold()).
const MAX_QUEUE = 20;
const QUEUE_STATES = new Set(['queued', 'steering']);
// References a chat keeps per email for the thread check (keepThread()); the newest are at the end.
const MAX_REFERENCES = 50;
// How a turn the user has not seen ended (watch()).
const UNREAD = ['reply', 'error'];
// What counts as a message for the chat list's order: what the user wrote, what the agent said, and an approval card,
// which asks the user something. Thinking, tool calls and settings such as the model leave the order alone.
const MESSAGE_TYPES = new Set(['user', 'assistant', 'approval']);
const SAVE_MS = 400;
// While a turn runs, items change all the time; saving all conversations every 400 ms would stall the main thread.
const SAVE_RUNNING_MS = 2000;
// Attachment copies older than this are left over from a crash or a forced quit.
const TEMP_MAX_AGE_MS = 24 * 60 * 60 * 1000;
const DELTA_MS = 50;
// An adapter that ignores stop() gets this long before the hub closes the turn itself.
const STOP_GRACE_MS = 12000;
const STATES = new Set(['ready', 'offline', 'unconfigured', 'disabled', 'missing', 'unauthorized', 'unknown']);
// An agent's model list is good for ten minutes. The panel asks again when it opens, which reloads a list older
// than half a minute; a list that failed is tried again after half a minute as well.
const MODELS_TTL_MS = 10 * 60 * 1000;
const MODELS_RETRY_MS = 30 * 1000;
const MAX_MODELS = 300;
// The agents' own skills, for the $ menu and for finding the ones a message names: kept a minute, and loaded again
// when the menu opens on a list older than ten seconds.
const OWN_SKILLS_TTL_MS = 60 * 1000;
const OWN_SKILLS_RETRY_MS = 10 * 1000;
// Message actions whose text Rukoo wrote (an approval) or that is a whole skill already: no $name in them counts.
const NO_MENTIONS = new Set(['approved', 'declined', 'skill']);
const ADAPTERS = { clark: ['./hermes', 'HermesAdapter'], claude: ['./claude', 'ClaudeAdapter'], codex: ['./codex', 'CodexAdapter'], openclaw: ['./openclaw', 'OpenClawAdapter'] };
// A custom agent's adapter, by its protocol (config.js CUSTOM_PRESETS).
const CUSTOM_ADAPTERS = { runs: ['./hermes', 'HermesAdapter'], openai: ['./openai', 'OpenAiAdapter'] };
const DENY_IDS = new Set(['deny', 'decline', 'cancel']);
// English fallbacks for error notices; the renderer localizes by code.
const ERROR_TEXT = {
  offline: 'Could not reach the agent.',
  unauthorized: 'The agent rejected the key or token.',
  'not-configured': 'The agent is not set up yet.',
  'not-installed': 'The agent program was not found.',
  disabled: 'The agent is turned off.',
  'spawn-failed': 'The agent program stopped unexpectedly.',
  busy: 'The agent is still working on the previous message.',
  window: "Rukoo's window is closed.",
  timeout: 'The agent did not respond in time.',
  protocol: 'The agent sent something Rukoo did not understand.',
  'rate-limited': 'The agent is rate limited. Try again in a moment.',
  stopped: 'Stopped.',
  unknown: 'Something went wrong.'
};

const rand = (n = 6) => crypto.randomBytes(n).toString('base64url').replace(/[-_]/g, '').slice(0, n).toLowerCase().padEnd(n, '0');
const newConversationId = () => `c_${Date.now().toString(36)}_${rand(6)}`;
const newItemId = () => `i_${rand(10)}`;
const newQueueId = () => `q_${rand(10)}`;
const clip = (v, max) => String(v == null ? '' : v).slice(0, max);
const sha256 = (s) => crypto.createHash('sha256').update(String(s)).digest();
const isObj = (v) => Boolean(v) && typeof v === 'object' && !Array.isArray(v);
// Files or emails added to a message that are no longer here. Only an error's message reaches the renderer, so it
// names them, as JSON (an email id can hold spaces): the renderer drops exactly those and keeps the rest.
function missingError(code, ids, detail) {
  const err = new AgentError(code, detail);
  err.message = `${code}: ${JSON.stringify(ids)}`;
  err.ids = ids;
  return err;
}
const normId = (v) => String(v || '').trim().replace(/^<|>$/g, '').toLowerCase();

// What a chat about a Rukoo Voice meeting keeps of it: the id, the title and linked event when the chat started, and
// readSeq, how far the agent has read the transcript (read_meeting_transcript's cursor, in the voice store's change
// numbers). null for anything else. paging, the places where reads that go on page by page stopped (up to eight), is
// left out: after a restart those reads start over and move the cursor no more.
function meetingRef(m) {
  if (!isObj(m) || !isMeetingId(m.id)) return null;
  return {
    id: m.id,
    title: clip(typeof m.title === 'string' ? m.title : '', 300),
    eventId: typeof m.eventId === 'string' && m.eventId ? clip(m.eventId, 4000) : null,
    readSeq: Number.isInteger(m.readSeq) && m.readSeq >= 0 ? m.readSeq : 0,
    ...(m.deleted === true ? { deleted: true } : {})
  };
}

// Anything thrown or reported becomes {code, detail} with a code from SPEC 4.6.
function toError(err) {
  if (!err) return { code: 'unknown', detail: '' };
  if (typeof err !== 'object') return { code: 'unknown', detail: clip(err, 500) };
  const known = typeof err.code === 'string' && Object.prototype.hasOwnProperty.call(ERROR_TEXT, err.code);
  const code = known ? err.code : 'unknown';
  const message = typeof err.message === 'string' && err.message !== err.code ? err.message : '';
  // Node system errors (ECONNREFUSED and friends) keep their code in the detail.
  const raw = !known && typeof err.code === 'string' ? err.code : '';
  return { code, detail: clip(err.detail || [raw, message].filter(Boolean).join(': '), 500) };
}

function withTimeout(promise, ms, code = 'timeout') {
  let timer;
  return Promise.race([
    Promise.resolve(promise).finally(() => clearTimeout(timer)),
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new AgentError(code, `no answer after ${ms} ms`)), ms);
    })
  ]);
}

// Finds the adapter class in a module written to SPEC 5, whatever it is exported as.
function adapterClass(mod, name) {
  if (typeof mod === 'function') return mod;
  if (mod && typeof mod[name] === 'function') return mod[name];
  const any = mod && Object.values(mod).find((v) => typeof v === 'function' && /Adapter$/.test(v.name || ''));
  if (any) return any;
  throw new Error(`${name} not exported`);
}

function emptyView() {
  return { openMessageId: null, checkedIds: [], scope: 'all', view: 'inbox', folder: null, composer: null };
}

class AgentHub extends EventEmitter {
  // listen: false skips the MCP listeners (unit tests that call tools directly).
  constructor({ engine, dataDir, secrets, deps = {}, adapters = null, listen = true } = {}) {
    super();
    this.engine = engine;
    this.dataDir = dataDir;
    this.deps = deps;
    this.listen = listen;
    this.cfg = new AgentConfig({ file: path.join(dataDir, 'agents.json'), secrets, interfaces: deps.interfaces });
    this.file = path.join(dataDir, 'conversations.json');
    this.workspace = deps.workspace || path.join(path.dirname(dataDir), 'agent-workspace');
    // The text of PDFs, from attachments and attached files, read in a process of its own. deps.pdf: its limits,
    // for tests.
    this.pdf = new PdfReader(deps.pdf);
    // Files the user attaches to messages: staged until sent, then in a folder per chat in the workspace.
    this.files = new ChatFiles(path.join(this.workspace, 'files'), this.pdf);
    // Rukoo's own skills ship in skills/ next to src/; the user's sit next to the data folder. Tests pass both.
    const skillDirs = deps.skills || {};
    this.skills = new skills.Skills({
      bundled: skillDirs.bundled !== undefined ? skillDirs.bundled : path.join(__dirname, '..', '..', '..', 'skills'),
      user: skillDirs.user !== undefined ? skillDirs.user : path.join(path.dirname(dataDir), 'skills'),
      // A skill the user wrote that Rukoo skips should be easy to find out about, so this logs without SEM_AGENT_LOG.
      log: deps.skillLog || ((line) => console.warn(`[agent:skills] ${line}`))
    });
    this.injectedAdapters = adapters;
    this.adapters = new Map();
    this.adapterErrors = new Map();
    this.conversations = new Map();
    this.turns = new Map();
    // Chats whose next message is still copying its files (send()): busy, as with a turn. Each with the send's
    // promise, which dispose() waits for, so a copy made while Rukoo closes is gone before it quits.
    this.sending = new Map();
    this.approvals = new Map();
    this.followUps = new Map();
    // How each chat's last turn ended ('done', 'stopped' or 'error'): only after 'done' does its queue move on.
    this.lastOutcome = new Map();
    this.tokens = [];
    this.convTokens = new Map();
    this.remoteHash = null;
    this.openclawHash = null;
    this.openclawRemoteHash = null;
    this.threads = new Map();
    this.uiPending = new Map();
    // Per conversation, the chain of mail work whose outcome goes into the notes, and what on it still runs; see
    // settleMail().
    this.mailSettling = new Map();
    this.mailRunning = new Map();
    // Attachment copies written for local agents (read_attachment local_path), removed on dispose.
    this.tempDirs = new Set();
    this.viewState = emptyView();
    // The chat the panel shows while it is open, or null. A turn that ends in any other chat leaves it unread (watch()).
    this.watching = null;
    // agent → { at, value, pending }: the models and efforts the panel offers (see models()).
    this.modelLists = new Map();
    this.skillLists = new Map();
    this.statuses = {};
    // Per agent, how many forced status probes have started; see probeStatus().
    this.statusRound = new Map(AGENT_IDS.map((id) => [id, 0]));
    this.customHashes = new Map();
    this.deltas = new Map();
    this.deltaTimer = null;
    this.saveTimer = null;
    this.mcp = null;
    this.remoteTimer = null;
    this.remoteStarting = false;
    this.remoteQueue = Promise.resolve();
    // Lets a paired phone run Claude Code and Codex turns here (phonehost.js); made in start(), after the config.
    this.phoneHost = null;
    // Links to a meeting's media for get_meeting_media_link, served on the MCP listeners (media.js). A test profile
    // (SEM_HIDDEN) may shorten their life with SEM_MEDIA_LINK_TTL_MS.
    const ttl = Number(process.env.SEM_MEDIA_LINK_TTL_MS);
    this.mediaLinks = new MediaLinks({
      voice: () => this.deps.voice || null,
      canRead: () => !this.meetingsOff(),
      agentAllowed: (agent, remote) => this.agentAllowed(agent, remote),
      tokenLive: (agent, remote, bind) => this.tokenLive(agent, remote, bind),
      endpoint: (remote) => this.mediaEndpoint(remote),
      ...(process.env.SEM_HIDDEN && Number.isSafeInteger(ttl) && ttl > 0 ? { ttlMs: ttl } : {}),
      log: (...a) => this.log('media', ...a)
    });
    this.started = false;
    this.disposed = false;
  }

  // ---------- lifecycle ----------

  // The synchronous part (config, conversations, adapters) runs before the first await, so renderer calls
  // that arrive while the MCP listeners start already see the stored conversations.
  async start() {
    if (this.started) return;
    this.started = true;
    this.cfg.load();
    this.phoneHost = new PhoneHost(this, this.deps.phone);
    this.load();
    this.meetingListener = (payload) => this.updateMeetingRefs(payload);
    this.deps.voice?.on?.('meeting', this.meetingListener);
    this.updateMeetingRefs();
    this.refreshRemoteHash();
    this.sweepTemp();
    this.registerAdapters();
    try {
      if (this.listen) await this.startMcp();
    } catch (err) {
      this.cfg.runtime.note = `MCP server did not start: ${err.message}`;
      this.log('hub', this.cfg.runtime.note);
    }
    this.refreshStatus().catch(() => {});
  }

  // In its long form: os.tmpdir() can be an 8.3 path (C:\Users\BARTAD~1\...). Claude gets this folder as
  // --add-dir, and a local_path written the short way would not match it, so every attachment would prompt.
  tempBase() {
    if (this.tempBaseDir) return this.tempBaseDir;
    const base = this.deps.tempDir || path.join(os.tmpdir(), 'rukoo-agent');
    let dir = base;
    try {
      fs.mkdirSync(base, { recursive: true });
      dir = fs.realpathSync.native(base);
    } catch (_) {
      // Not writable: writeTemp fails on its own and read_attachment goes without local_path.
    }
    this.tempBaseDir = dir;
    return dir;
  }

  // dispose() removes this run's attachment copies; a crash or a forced quit leaves them behind. Only old
  // folders go, so a second instance (dev, tests) keeps the files it is using.
  sweepTemp() {
    const base = this.tempBase();
    let names;
    try {
      names = fs.readdirSync(base);
    } catch (_) {
      return;
    }
    const cutoff = Date.now() - TEMP_MAX_AGE_MS;
    for (const name of names) {
      const dir = path.join(base, name);
      try {
        if (fs.statSync(dir).mtimeMs < cutoff) fs.rmSync(dir, { recursive: true, force: true });
      } catch (_) {
        // In use or already gone.
      }
    }
  }

  registerAdapters() {
    for (const id of this.cfg.agentIds()) this.registerAdapter(id);
  }

  // Custom agents come and go while Rukoo runs: adds the adapter of a new one and closes that of a removed one. The
  // chats of a removed agent stay; they can't send until an agent with that id is back, which none will be.
  syncCustomAdapters() {
    const ids = new Set(this.cfg.agentIds());
    for (const id of ids) if (!this.adapters.has(id) && !this.adapterErrors.has(id)) this.registerAdapter(id);
    for (const id of [...this.adapters.keys(), ...this.adapterErrors.keys()]) {
      if (ids.has(id) || !isCustomId(id)) continue;
      const a = this.adapters.get(id);
      this.adapters.delete(id);
      this.adapterErrors.delete(id);
      this.statusRound.delete(id);
      const { [id]: _gone, ...rest } = this.statuses || {};
      this.statuses = rest;
      if (a && typeof a.dispose === 'function') Promise.resolve().then(() => a.dispose()).catch(() => {});
    }
  }

  registerAdapter(id) {
    if (!this.statusRound.has(id)) this.statusRound.set(id, 0);
    const custom = isCustomId(id) ? this.cfg.custom(id) : null;
    const opts = {
      id,
      ...(custom ? { custom: true } : {}),
      config: () => this.cfg.get(id),
      hub: this,
      log: (...args) => this.log(id, ...args),
      // attachments: where read_attachment saves files for local agents, so Claude may read them without asking.
      paths: { workspace: this.workspace, attachments: this.tempBase() }
    };
    try {
      let adapter;
      if (this.injectedAdapters) {
        // Tests give a custom agent's adapter by its id, or one for every custom agent of a protocol.
        const given = this.injectedAdapters[id] || (custom && this.injectedAdapters[`custom:${custom.protocol}`]);
        if (!given) return;
        adapter = typeof given.runTurn === 'function' ? given : given(opts);
      } else if (process.env.SEM_AGENT_FAKE === '1') {
        const { FakeAdapter } = require('./fake');
        adapter = new FakeAdapter(opts);
      } else {
        // Each adapter loads on its own, so one broken or missing module only takes out that agent.
        const [file, name] = custom ? CUSTOM_ADAPTERS[custom.protocol] : ADAPTERS[id];
        const Cls = adapterClass(require(file), name);
        adapter = new Cls(opts);
      }
      this.adapters.set(id, adapter);
    } catch (err) {
      this.adapterErrors.set(id, clip(err && err.message, 300));
      this.log(id, 'adapter not loaded:', err && err.message);
    }
  }

  log(scope, ...args) {
    if (process.env.SEM_AGENT_LOG || process.env.SEM_TRACE) console.log(`[agent:${scope}]`, ...args);
  }

  async startMcp() {
    this.mcp = new McpServer({ hub: this, version: this.deps.appVersion || '0.0.0', log: (...a) => this.log('mcp', ...a) });
    const want = this.cfg.data.mcp.port;
    let note = '';
    try {
      await this.mcp.listen({ host: '127.0.0.1', port: want, remote: false });
    } catch (err) {
      if (err.code !== 'EADDRINUSE' && err.code !== 'EACCES') throw err;
      await this.mcp.listen({ host: '127.0.0.1', port: 0, remote: false });
      note = `Port ${want} is in use; Rukoo uses ${this.mcp.ports().local} for local agents.`;
    }
    this.cfg.runtime.localPort = this.mcp.ports().local;
    this.cfg.runtime.note = note;
    await this.startRemote();
    // Tailscale often comes up after Rukoo (login items); keep trying while the user wants remote access.
    clearInterval(this.remoteTimer);
    this.remoteTimer = setInterval(() => {
      if (this.disposed || !this.mcp || !this.wantsRemote() || this.mcp.ports().remote || this.remoteStarting) return;
      this.remoteStarting = true;
      this.startRemote()
        .then(() => this.mcp && this.mcp.ports().remote && this.refreshStatus({ force: true }))
        .catch(() => {})
        .finally(() => {
          this.remoteStarting = false;
        });
    }, 60000);
    if (this.remoteTimer.unref) this.remoteTimer.unref();
  }

  // One start at a time. A start still waiting for its listener (the retry timer, say) would otherwise add
  // it after a later start had closed the remote listeners because the user turned remote access off.
  startRemote() {
    const run = this.remoteQueue.then(() => this.openRemote());
    this.remoteQueue = run.catch(() => {});
    return run;
  }

  // Whether the Tailscale listener should be open: for Clark's bridge ("Let Clark use Rukoo"), for an OpenClaw
  // gateway on another computer ("Let OpenClaw use Rukoo from another computer", while OpenClaw is on), or for a
  // paired phone ("Let my phone use this computer"). Each is an explicit opt-in, because the listener offers the
  // mailbox to the tailnet. The phone's switch opens only the /phone/ routes: /mcp still takes Clark's and
  // OpenClaw's tokens only while their own switches are on (identify()).
  wantsRemote() {
    const { mcp, openclaw, phone, custom } = this.cfg.data;
    const customRemote = (custom || []).some((c) => c.remote && c.enabled !== false);
    return Boolean(mcp.remote || (openclaw.remote && openclaw.enabled !== false) || (phone && phone.enabled) || customRemote);
  }

  // The Tailscale listener, only when the user turned it on and this machine has an address.
  async openRemote() {
    const mcp = this.mcp;
    if (!mcp || mcp.closed) return;
    await mcp.closeRemote();
    this.cfg.runtime.remotePort = null;
    const { remoteHost, port } = this.cfg.data.mcp;
    if (!this.wantsRemote()) return;
    const detected = tailscaleAddress(this.cfg.interfaces);
    const literal = /^\d+\.\d+\.\d+\.\d+$/.test(remoteHost);
    const host = literal ? remoteHost : detected || remoteHost;
    if (!host) {
      this.cfg.runtime.note = 'No Tailscale address found on this PC.';
      return;
    }
    const names = remoteHost && remoteHost !== host ? [remoteHost] : [];
    try {
      await mcp.listen({ host, port, remote: true, hosts: names });
    } catch (err) {
      // dispose() or a port change closed this server meanwhile; the note belongs to the server that replaced it.
      if (mcp.closed) return;
      if (err.code !== 'EADDRINUSE') {
        this.cfg.runtime.note = `Could not listen on ${host}:${port}: ${err.code || err.message}`;
        return;
      }
      try {
        await mcp.listen({ host, port: mcp.ports().local || 0, remote: true, hosts: names });
      } catch (err2) {
        if (!mcp.closed) this.cfg.runtime.note = `Could not listen on ${host}: ${err2.code || err2.message}`;
        return;
      }
    }
    this.cfg.runtime.remotePort = mcp.ports().remote;
    if (this.cfg.runtime.remotePort !== port) this.cfg.runtime.note = `Port ${port} is in use on ${host}; Rukoo uses ${this.cfg.runtime.remotePort}.`;
  }

  async dispose() {
    if (this.disposed) return;
    this.disposed = true;
    if (this.meetingListener) this.deps.voice?.off?.('meeting', this.meetingListener);
    clearInterval(this.remoteTimer);
    // PDF reads still running end now, so no reader process outlives Rukoo; their turns stop below.
    this.pdf.dispose();
    this.files.dispose();
    // Phone turns end with done/stopped on their streams, before the adapters below close their processes.
    if (this.phoneHost) this.phoneHost.dispose();
    for (const turn of [...this.turns.values()]) {
      // runTurn() would give these back only after the flush below, so a turn the agent does not have yet gives
      // them back now.
      const c = this.conversations.get(turn.cid);
      if (c && !turn.reached) this.giveBack(c, turn);
      turn.controller.abort();
      this.finishTurn(turn, { status: 'stopped' });
    }
    for (const [id, p] of this.uiPending) {
      clearTimeout(p.timer);
      p.reject(new AgentError('window', 'Rukoo is closing'));
      this.uiPending.delete(id);
    }
    // Approvals queued behind a turn that ended just now: the agent hears about them with the next message.
    for (const [cid, queue] of this.followUps) {
      const c = this.conversations.get(cid);
      if (c) for (const message of queue) this.keepApproval(c, message);
    }
    this.followUps.clear();
    // Mail work still running or queued: its outcome would come too late to be saved, so the agent hears that it is
    // not known.
    for (const [cid, running] of this.mailRunning) {
      const c = this.conversations.get(cid);
      if (!c) continue;
      for (const { label, kind } of running) {
        c.notes.push(
          kind === 'calendar'
            ? `Rukoo closed while ${context.unsafeInline(label || 'an event', 'calendar')} was still being added, so its outcome is not known. Check the calendar with list_events before you ask again.`
            : `Rukoo closed while ${context.unsafeInline(label || 'a mail action', 'mail action')} was still running, so its outcome is not known. Check the mail before you act on it again.`
        );
      }
    }
    clearTimeout(this.deltaTimer);
    this.deltas.clear();
    // Synchronous, so a quit that does not wait for this promise still keeps the transcript.
    this.flush();
    for (const dir of this.tempDirs) {
      try {
        fs.rmSync(dir, { recursive: true, force: true });
      } catch (_) {
        // A file an agent still holds open; the OS cleans temp eventually.
      }
    }
    const closing = [...this.adapters.values()].map((a) => Promise.resolve().then(() => a.dispose && a.dispose()));
    if (this.mcp) closing.push(this.mcp.close());
    // Messages still copying their files: they stop at the next file and remove their copies (send()). A quit that
    // can't wait that long leaves them to the next start (ChatFiles.prune).
    closing.push(...this.sending.values());
    await Promise.allSettled(closing);
  }

  // ---------- persistence ----------

  load() {
    let raw;
    let broken = false;
    // A file Rukoo can't read, or can read only in part, is kept aside instead of being overwritten on the next
    // save. When none of it loads it moves; when some does, a copy stays.
    const keepAside = (move) => {
      broken = true;
      try {
        (move ? fs.renameSync : fs.copyFileSync)(this.file, `${this.file}.bad-${Date.now()}`);
      } catch (_) {
        // Nothing to keep.
      }
    };
    try {
      raw = JSON.parse(fs.readFileSync(this.file, 'utf8'));
    } catch (err) {
      if (err.code !== 'ENOENT') keepAside(true);
    }
    // Valid JSON is not yet a store: {"conversations": null} would load as no chats at all.
    if (raw !== undefined && !(isObj(raw) && Array.isArray(raw.conversations))) keepAside(true);
    const list = !broken && raw ? raw.conversations : [];
    for (const c of list) {
      // A chat with a custom agent the user removed stays, as a chat that can't send.
      if (!isObj(c) || typeof c.id !== 'string' || !(this.cfg.hasAgent(c.agent) || isCustomId(c.agent)) || !Array.isArray(c.items)) {
        if (!broken) keepAside(false);
        continue;
      }
      c.status = 'idle';
      c.provider = isObj(c.provider) ? c.provider : {};
      c.notes = Array.isArray(c.notes) ? c.notes.filter((n) => typeof n === 'string') : [];
      // Saved before Rukoo tracked this: a chat with a message the agent answered has had its first turn.
      if (typeof c.delivered !== 'boolean') c.delivered = c.items.some((i) => i.type === 'assistant');
      c.needsRecap = c.needsRecap === true;
      // Kept as how the turn ended, since loading sets every status back to idle. Saved before Rukoo tracked it: read.
      c.unread = UNREAD.includes(c.unread) ? c.unread : false;
      // Saved before chats could be continued from a newer email: none.
      c.continued = Array.isArray(c.continued) ? c.continued.filter((e) => isObj(e) && typeof e.id === 'string' && e.id).map((e) => ({ ...e, pending: e.pending === true })) : [];
      // Saved before chats had a model and effort of their own: they follow the agent's default.
      c.model = typeof c.model === 'string' && c.model ? c.model : null;
      c.effort = EFFORTS.includes(c.effort) ? c.effort : null;
      // Saved before chats had a permission level: they follow the one in Settings, and the agent heard of none.
      c.permission = permissionLevel(c.permission);
      c.toldPermission = permissionLevel(c.toldPermission);
      // Saved before files could be attached: none.
      c.files = Array.isArray(c.files) ? c.files.filter((f) => isObj(f) && typeof f.id === 'string' && typeof f.file === 'string') : [];
      // Saved before chats could be about a meeting: none.
      c.meeting = meetingRef(c.meeting);
      // Messages that waited when Rukoo quit wait again, until the user sends one: the turn they waited for is gone.
      c.queue = cleanQueue(c.queue);
      // A turn cannot survive a restart: close whatever was still open.
      for (const item of c.items) {
        if (item.type === 'assistant' && item.status === 'streaming') item.status = 'stopped';
        if (item.type === 'thinking' && item.status === 'running') Object.assign(item, { status: 'done', endedAt: item.endedAt || c.updatedAt });
        if (item.type === 'tool' && item.status === 'running') Object.assign(item, { status: 'done', endedAt: item.endedAt || c.updatedAt });
        if (item.type === 'approval' && item.status === 'pending' && item.kind === 'runtime') item.status = 'expired';
        // The engine keeps what Undo needs in memory only, so a mail notice's Undo ends with the app as well.
        if (item.type === 'notice' && item.undo) item.undo = null;
      }
      this.conversations.set(c.id, c);
      if (c.provider.threadId) this.threads.set(String(c.provider.threadId), c.id);
    }
    // The files of chats that are gone. Not while a file Rukoo couldn't read is kept aside: its chats may come
    // back, and their files with them. After the next save the store alone would say they are gone.
    // A waiting message's files are in the chat's folder already.
    if (!broken && !this.keptAside()) this.files.prune([...this.conversations.values()].map((c) => [c.id, [...c.files, ...c.queue.flatMap((e) => e.files)]]));
  }

  keptAside() {
    const prefix = `${path.basename(this.file)}.bad-`;
    try {
      return fs.readdirSync(path.dirname(this.file)).some((name) => name.startsWith(prefix));
    } catch (_) {
      return true;
    }
  }

  scheduleSave() {
    // After dispose() flushed, nothing more is written: the app is quitting.
    if (this.saveTimer || this.disposed) return;
    this.saveTimer = setTimeout(
      () => {
        this.saveTimer = null;
        this.save();
      },
      this.turns.size ? SAVE_RUNNING_MS : SAVE_MS
    );
  }

  save() {
    try {
      const conversations = [...this.conversations.values()].sort((a, b) => b.updatedAt - a.updatedAt);
      writeJsonAtomic(this.file, { version: 1, conversations });
    } catch (err) {
      this.log('hub', 'saving conversations failed:', err.message);
    }
  }

  flush() {
    clearTimeout(this.saveTimer);
    this.saveTimer = null;
    this.save();
  }

  // ---------- events ----------

  emitEvent(payload) {
    if (payload.kind !== 'delta') this.flushDeltas();
    this.emit('event', payload);
  }

  // Streaming text arrives in tiny pieces; send it to the renderer at most every 50 ms per item.
  queueDelta(cid, itemId, text) {
    const key = `${cid}\n${itemId}`;
    const d = this.deltas.get(key);
    if (d) d.text += text;
    else this.deltas.set(key, { conversationId: cid, itemId, text });
    if (!this.deltaTimer) this.deltaTimer = setTimeout(() => this.flushDeltas(), DELTA_MS);
  }

  flushDeltas() {
    clearTimeout(this.deltaTimer);
    this.deltaTimer = null;
    if (!this.deltas.size) return;
    const list = [...this.deltas.values()];
    this.deltas.clear();
    for (const d of list) this.emit('event', { kind: 'delta', conversationId: d.conversationId, itemId: d.itemId, field: 'text', text: d.text });
  }

  emitConversation(c) {
    this.emitEvent({ kind: 'conversation', conversation: c });
  }

  updateMeetingRefs(payload = null) {
    const voice = this.deps.voice;
    if (!voice) return;
    const id = payload?.id || payload?.meeting?.id;
    let changed = false;
    for (const c of this.conversations.values()) {
      if (!c.meeting || (id && c.meeting.id !== id)) continue;
      const m = voice.meeting?.(c.meeting.id);
      const title = m?.meeting.title || c.meeting.title;
      const deleted = voice.isDeleted?.(c.meeting.id) === true || m?.meeting.removed === true || (payload?.kind === 'removed' && id === c.meeting.id);
      if (c.meeting.title === title && (c.meeting.deleted === true) === deleted) continue;
      c.meeting = { ...c.meeting, title };
      if (deleted) c.meeting.deleted = true;
      else delete c.meeting.deleted;
      this.emitEvent({ kind: 'conversation', conversation: { id: c.id, meeting: c.meeting } });
      changed = true;
    }
    if (changed) this.scheduleSave();
  }

  touch(c) {
    c.updatedAt = Date.now();
    this.scheduleSave();
  }

  addItem(c, fields) {
    const item = { id: newItemId(), at: Date.now(), ...fields };
    for (const k of ['text', 'detail', 'summary']) if (typeof item[k] === 'string') item[k] = clip(item[k], MAX_TEXT);
    c.items.push(item);
    const removed = this.trimItems(c, item);
    this.touch(c);
    this.emitEvent({ kind: 'item', conversationId: c.id, item });
    // Only the ids, so a long transcript is not sent again for every new item.
    if (removed.length) this.emitEvent({ kind: 'trim', conversationId: c.id, itemIds: removed });
    return item;
  }

  // Drops the oldest items past MAX_ITEMS. An approval that still waits for the user stays, so nobody is
  // left waiting on a card that is gone. Only when nothing else is left does the oldest one expire and go.
  trimItems(c, keep) {
    const removed = [];
    while (c.items.length > MAX_ITEMS) {
      let i = c.items.findIndex((x) => x !== keep && !(x.type === 'approval' && x.status === 'pending'));
      if (i < 0) {
        i = 0;
        const w = this.approvals.get(c.items[0].id);
        this.approvals.delete(c.items[0].id);
        if (w) w.resolve('cancel');
      }
      removed.push(c.items.splice(i, 1)[0].id);
    }
    return removed;
  }

  updateItem(c, item) {
    this.touch(c);
    this.emitEvent({ kind: 'item', conversationId: c.id, item });
    return item;
  }

  setStatus(c, status, error = null) {
    c.status = status;
    this.touch(c);
    this.emitEvent({ kind: 'status', conversationId: c.id, status, ...(error ? { error } : {}) });
  }

  // ---------- conversations (renderer API) ----------

  mustGet(id) {
    const c = this.conversations.get(id);
    if (!c) throw new AgentError('invalid', 'unknown conversation');
    return c;
  }

  agentName(agent) {
    return this.cfg.displayName(agent);
  }

  // Newest message first, for the panel's chat list (agent/panel.js chatItems).
  list() {
    return [...this.conversations.values()]
      .map((c) => ({ c, lastAt: lastMessageAt(c) }))
      .sort((a, b) => b.lastAt - a.lastAt)
      .map(({ c, lastAt }) => {
        const last = [...c.items].reverse().find((i) => (i.type === 'assistant' && !i.interim) || i.type === 'user');
        // A message with only attachments reads as their names.
        const text = last ? last.text || [...(last.files || []).map((f) => f.name), ...(last.emails || []).map((e) => e.subject)].join(', ') : '';
        return {
          id: c.id,
          agent: c.agent,
          title: c.title,
          message: c.message,
          meeting: c.meeting ? { id: c.meeting.id, title: c.meeting.title, ...(c.meeting.deleted ? { deleted: true } : {}) } : null,
          origin: c.origin,
          updatedAt: c.updatedAt,
          lastAt,
          status: c.status,
          // 'reply' or 'error': how the turn the user has not seen ended. null when they have seen it.
          unread: c.unread || null,
          // An approval card in it waits for the user.
          waiting: c.items.some((i) => i.type === 'approval' && i.status === 'pending'),
          preview: clip(String(text || '').replace(/\s+/g, ' ').trim(), 140)
        };
      });
  }

  // The panel shows this chat (null: none, or the panel is closed). Showing it reads its answer.
  watch(id) {
    const c = id ? this.conversations.get(id) : null;
    this.watching = c ? c.id : null;
    if (c && c.unread) this.markUnread(c, false);
    return true;
  }

  // unread: false, 'reply' or 'error'. Without touching updatedAt.
  markUnread(c, unread) {
    if (c.unread === unread) return;
    c.unread = unread;
    this.scheduleSave();
    this.emitEvent({ kind: 'conversation', conversation: { id: c.id, unread } });
  }

  get(id) {
    this.flushDeltas();
    return this.conversations.get(id) || null;
  }

  // The chat about this email, or the one the user continued from it (continueFrom()); the most recent of them.
  findFor(ref = {}) {
    const want = normId(ref.messageId);
    const id = ref.id ? String(ref.id) : '';
    // The same email can arrive in two accounts; each copy has its own chats. A saved copy has no account.
    const account = ref.accountId || tools.accountOfId(id);
    const about = (m) => {
      if (!isObj(m)) return false;
      const own = m.accountId || tools.accountOfId(m.id);
      return want && m.messageId ? normId(m.messageId) === want && !(account && own && own !== account) : Boolean(id) && m.id === id;
    };
    let best = null;
    for (const c of this.conversations.values()) {
      const hit = about(c.message) || (c.continued || []).some(about);
      if (hit && (!best || c.updatedAt > best.updatedAt)) best = c;
    }
    return best;
  }

  // The most recent chat about this Rukoo Voice meeting, or null.
  findForMeeting(id) {
    let best = null;
    for (const c of this.conversations.values()) {
      if (c.meeting && c.meeting.id === id && (!best || c.updatedAt > best.updatedAt)) best = c;
    }
    return best;
  }

  // For an email without a chat: the most recent chat about an earlier email in its thread, in the same account,
  // which the panel offers to continue. Only a header link counts (see tools.earlierInThread). The renderer passes
  // the open email's In-Reply-To and References; the folder cache has In-Reply-To only.
  relatedFor(ref = {}) {
    const id = ref.id ? String(ref.id) : '';
    const cached = id ? this.liveMessage(id) : null;
    const account = tools.accountOfId(id) || (cached && cached.accountId) || ref.accountId || null;
    if (!account) return null;
    const email = {
      messageId: ref.messageId || (cached && cached.messageId) || null,
      inReplyTo: ref.inReplyTo || (cached && cached.inReplyTo) || null,
      references: Array.isArray(ref.references) ? ref.references : [],
      date: Number(ref.date) || (cached && cached.date) || 0
    };
    const chats = [...this.conversations.values()];
    const earlier = tools.earlierInThread(this.engine, email, account, chats.flatMap((c) => this.chatEmails(c, account)));
    if (!earlier.size) return null;
    let best = null;
    for (const c of chats) {
      if (this.inThread(c, earlier, account) && (!best || c.updatedAt > best.updatedAt)) best = c;
    }
    if (!best || !best.message) return null;
    const m = best.message;
    return { id: best.id, agent: best.agent, title: best.title, updatedAt: best.updatedAt, message: { subject: m.subject || '', from: m.from || null, date: m.date || null } };
  }

  // A chat's emails in this account: its own and the ones it was continued from. With the headers the chat kept
  // (keepThread()), they count for a thread also when they have left the folder cache.
  chatEmails(c, account) {
    if (!c.message) return [];
    return [c.message, ...(c.continued || [])].filter((m) => isObj(m) && m.messageId && (m.accountId || tools.accountOfId(m.id)) === account);
  }

  // Whether one of a chat's emails in this account has a Message-ID in earlier (tools.earlierInThread). A reply to
  // the email a chat was continued from belongs to that chat as well.
  inThread(c, earlier, account) {
    return this.chatEmails(c, account).some((m) => earlier.has(normId(m.messageId)));
  }

  // Keeps the thread headers Rukoo read for an email a chat keeps, so a reply to the same email still finds the chat
  // once this one moved to a folder Rukoo has not opened. Only from a message Rukoo loaded itself, which has its
  // References; the folder cache gives In-Reply-To alone (messageRef). Returns whether anything changed.
  keepThread(ref, full) {
    if (!isObj(ref) || !isObj(full)) return false;
    const inReplyTo = full.inReplyTo ? clip(full.inReplyTo, 1000) : null;
    const references = [].concat(full.references || []).slice(-MAX_REFERENCES).map((v) => clip(v, 1000)).filter(Boolean);
    if ((ref.inReplyTo || null) === inReplyTo && JSON.stringify(ref.references || []) === JSON.stringify(references)) return false;
    ref.inReplyTo = inReplyTo;
    ref.references = references;
    return true;
  }

  // What Rukoo itself knows of an email that is here, for the thread check: the account its id names (a saved copy's
  // entry names it), and its headers from the stored message, or the folder cache if that does not load. Nothing
  // from the renderer. null when the email is not here.
  async threadEmail(id) {
    const cached = this.liveMessage(id);
    if (!cached) return null;
    const account = String(id).startsWith('saved:') ? cached.accountId || null : tools.accountOfId(id);
    if (!account) return null;
    const full = await this.engine.getMessage(id).catch(() => null);
    const src = full || cached;
    return { account, full, email: { messageId: src.messageId || null, inReplyTo: src.inReplyTo || null, references: full ? full.references || [] : [], date: src.date || 0 } };
  }

  // The user continues a chat from a newer email in its thread. The chat keeps its own email. The newer one goes to
  // the agent with the next message that reaches it (runTurn()), and opening that email again shows this chat
  // (findFor()) instead of offering it again. Only the email's id comes from the renderer: Rukoo checks with what it
  // knows itself that the email is in the chat's account and thread, so no other email can be tied to the chat.
  async continueFrom(id, input) {
    const c = this.mustGet(id);
    const ref = this.messageRef(isObj(input) ? { id: input.id } : null);
    const known = ref ? await this.threadEmail(ref.id) : null;
    if (!known) throw new AgentError('invalid', 'no such email to continue from');
    if (this.conversations.get(c.id) !== c) throw new AgentError('invalid', 'unknown conversation');
    const same = (a, b) => isObj(a) && (a.messageId && b.messageId ? normId(a.messageId) === normId(b.messageId) : a.id === b.id);
    if (same(c.message, ref)) return c;
    const earlier = tools.earlierInThread(this.engine, known.email, known.account, this.chatEmails(c, known.account));
    if (!this.inThread(c, earlier, known.account)) throw new AgentError('invalid', "that email is not a newer one in this chat's thread");
    const entry = { ...ref, messageId: known.email.messageId || ref.messageId, at: Date.now(), pending: true };
    this.keepThread(entry, known.full);
    const list = (c.continued || []).filter((e) => !same(e, ref));
    list.push(entry);
    c.continued = this.pruneContinued(list);
    this.touch(c);
    this.emitContinued(c);
    return c;
  }

  // Past MAX_CONTINUED the oldest emails the agent has had go. One it has not had yet stays until a message takes it.
  pruneContinued(list) {
    for (let i = 0; list.length > MAX_CONTINUED && i < list.length; ) {
      if (list[i].pending) i++;
      else list.splice(i, 1);
    }
    return list;
  }

  // Without the transcript: the panel only needs the list for the email chip (agent/panel.js newerOpen).
  emitContinued(c) {
    this.emitEvent({ kind: 'conversation', conversation: { id: c.id, continued: c.continued, updatedAt: c.updatedAt } });
  }

  // The current Rukoo id of an email a card points at: the stored id while it still holds that email, else the
  // same Message-ID within the same account (mail moves within its account). One Message-ID can be several
  // emails to act on (Inbox and Sent of a mail to yourself, two Gmail labels), so in this order: the one copy in
  // the folder the stale id named (a new UID), where Rukoo moved the copy from that folder, the one live copy
  // in the synced folders. Several there and none of these: not clear which, so none. null when it is gone.
  async locate(ref = {}) {
    const id = ref.id ? String(ref.id) : '';
    const header = ref.messageHeader ? String(ref.messageHeader) : '';
    const cached = id ? tools.cacheMessage(this.engine, id) : null;
    if (cached && (!header || !cached.messageId || normId(cached.messageId) === normId(header))) return id;
    // A saved copy is there until the user deletes it. After that its id is as stale as a moved email's:
    // the Message-ID may still find the email in the mailbox, else it is gone.
    if (id.startsWith('saved:') && this.engine.saved.some((s) => s.id === id)) return id;
    if (!header) return null;
    const account = ref.accountId || tools.accountOfId(id);
    let folder = null;
    try {
      folder = id && !id.startsWith('saved:') ? decodeId(id).folder : null;
    } catch (_) {
      folder = null;
    }
    const copies = tools.copiesOf(this.engine, header).filter((h) => !account || h.acc.id === account);
    const live = copies.filter((h) => h.role !== 'all');
    const same = live.filter((h) => h.folder === folder);
    if (same.length === 1) return same[0].id;
    if (account) {
      try {
        const moved = await this.engine.findMoved(account, header, folder);
        if (moved) return moved;
      } catch (err) {
        this.log('hub', 'finding a moved email failed:', err.message);
      }
    }
    if (live.length > 1) return null;
    return (live[0] || copies[0] || {}).id || null;
  }

  // Normalizes the email reference the renderer sends, filling gaps from the engine cache.
  messageRef(input) {
    if (!isObj(input) || !input.id) return null;
    const id = clip(input.id, 2000);
    const ref = {
      id,
      messageId: input.messageId ? clip(input.messageId, 1000) : null,
      accountId: input.accountId ? clip(input.accountId, 200) : null,
      subject: clip(input.subject || '', 1000),
      from: isObj(input.from) ? { name: clip(input.from.name || '', 200), address: clip(input.from.address || '', 320) } : null,
      date: Number(input.date) || null,
      // Keeps the /unsubscribe command for the chat once the email is no longer on screen.
      unsubscribe: input.unsubscribe === true
    };
    const cached = this.liveMessage(id);
    if (cached) {
      if (!ref.unsubscribe && cached.unsubscribe) ref.unsubscribe = true;
      if (!ref.messageId && cached.messageId) ref.messageId = cached.messageId;
      if (!ref.subject) ref.subject = cached.subject || '';
      if (!ref.from && cached.from) ref.from = { name: cached.from.name || '', address: cached.from.address || '' };
      if (!ref.date) ref.date = cached.date || null;
      // Kept for the thread check, from Rukoo's own cache (see keepThread()).
      if (cached.inReplyTo) ref.inReplyTo = clip(cached.inReplyTo, 1000);
      // A saved copy's id names no account; its entry does.
      if (!ref.accountId) ref.accountId = id.startsWith('saved:') ? cached.accountId || null : id.split(':').map(decodeURIComponent)[0];
    }
    return ref;
  }

  // meeting: {id, title, eventId} for a chat about a Rukoo Voice meeting, which has no email.
  create({ agent, message = null, meeting = null, origin = 'panel', title = '', model = null, effort = null, permission = null } = {}) {
    const id = this.cfg.hasAgent(agent) ? agent : this.cfg.data.defaultAgent;
    if (!this.cfg.enabled(id)) throw new AgentError('disabled', `${id} is turned off`);
    const about = meetingRef(meeting);
    const ref = about ? null : this.messageRef(message);
    const now = Date.now();
    // Agents get the chat's title (Hermes names its session after it). With agents' access to meetings off, a
    // meeting chat is named after its first message instead, as a chat without an email is.
    const meetingTitle = about && about.title && !this.meetingsOff() ? about.title : '';
    const c = {
      id: newConversationId(),
      agent: id,
      title: clip(ref && ref.subject ? ref.subject : meetingTitle || title, 80),
      message: ref,
      // The meeting this chat is about, with how far the agent read its transcript (read_meeting_transcript).
      meeting: about ? { ...about, readSeq: 0 } : null,
      origin: origin === 'external' ? 'external' : 'panel',
      createdAt: now,
      updatedAt: now,
      status: 'idle',
      // The model and effort for this chat, picked in the panel; null follows the agent's default (setChoice()).
      model: null,
      effort: null,
      // The permission level for this chat, picked in the panel; null follows Settings (permissionOf()).
      permission: null,
      // The level the agent was last told about in a turn (runTurn()); null: none yet.
      toldPermission: null,
      provider: {},
      notes: [],
      // Set once a turn reached the agent; until then every message is a first turn with the email attached.
      delivered: false,
      // Set when the agent lost its session and started a new one; until a turn reaches that one, every message
      // carries the email and a recap of the chat (see startOver()).
      needsRecap: false,
      // 'reply' or 'error' when a turn ended while the chat was not on screen, until the user opens it (watch()).
      unread: false,
      // Newer emails in the same thread the user continued this chat from, oldest first (see continueFrom()).
      continued: [],
      // Files the user attached to its messages, in the chat's folder (chatfiles.js); read_chat_file reads them.
      files: [],
      // Messages the user sent while the agent worked, oldest first, that are not in the transcript yet (hold()).
      queue: [],
      items: []
    };
    // Picked before the chat started.
    this.applyChoice(c, { model, effort, permission });
    this.conversations.set(c.id, c);
    this.enforceLimit(c);
    this.scheduleSave();
    this.emitConversation(c);
    return c;
  }

  // Keeps at most 150 panel conversations and 20 external ones, each counted on its own, so an agent calling
  // in from outside can never push out the user's own chats. The oldest idle ones go first; keep is the new one.
  enforceLimit(keep) {
    const external = keep.origin === 'external';
    const max = external ? MAX_EXTERNAL : MAX_CONVERSATIONS;
    const same = [...this.conversations.values()].filter((c) => (c.origin === 'external') === external);
    const idle = same.filter((c) => c !== keep && !this.turns.has(c.id) && !this.sending.has(c.id)).sort((a, b) => a.updatedAt - b.updatedAt);
    for (let n = same.length; n > max && idle.length; n--) this.drop(idle.shift(), true);
  }

  remove(id) {
    const c = this.conversations.get(id);
    if (!c) return false;
    this.drop(c, false);
    this.scheduleSave();
    return true;
  }

  drop(c, quiet) {
    const turn = this.turns.get(c.id);
    if (turn) {
      turn.controller.abort();
      this.finishTurn(turn, { status: 'stopped' });
    }
    for (const [itemId, w] of this.approvals) {
      if (w.cid !== c.id) continue;
      this.approvals.delete(itemId);
      w.resolve('cancel');
    }
    this.followUps.delete(c.id);
    this.lastOutcome.delete(c.id);
    const token = this.convTokens.get(c.id);
    if (token) this.revokeToken(token);
    this.convTokens.delete(c.id);
    for (const [threadId, cid] of this.threads) if (cid === c.id) this.threads.delete(threadId);
    this.conversations.delete(c.id);
    if (this.watching === c.id) this.watching = null;
    this.files.remove(c.id);
    const adapter = this.adapters.get(c.agent);
    // Optional adapter hook: lets Claude kill the process that belongs to this conversation.
    if (adapter && typeof adapter.forget === 'function') Promise.resolve().then(() => adapter.forget(c)).catch(() => {});
    this.emitEvent({ kind: 'conversation', conversation: { id: c.id, removed: true } });
    if (!quiet) this.log('hub', 'removed', c.id);
  }

  view(state) {
    const s = isObj(state) ? state : {};
    const str = (v, max) => (typeof v === 'string' && v ? v.slice(0, max) : null);
    const composer = isObj(s.composer)
      ? { mode: str(s.composer.mode, 20), replyToId: str(s.composer.replyToId, 2000), draftId: str(s.composer.draftId, 2000) }
      : null;
    this.viewState = {
      mode: ['mail', 'calendar', 'meeting'].includes(s.mode) ? s.mode : 'mail',
      meetingId: str(s.meetingId, 80),
      openMessageId: str(s.openMessageId, 2000),
      checkedIds: (Array.isArray(s.checkedIds) ? s.checkedIds : []).filter((v) => typeof v === 'string').slice(0, 50).map((v) => v.slice(0, 2000)),
      scope: str(s.scope, 200) || 'all',
      view: str(s.view, 40) || 'inbox',
      folder: str(s.folder, 1000),
      composer
    };
    return true;
  }

  // ---------- attachments (renderer API) ----------

  // A file for the message the user is writing, from main's own file dialog or as the bytes the renderer dropped
  // or pasted. {id, name, size, type, kind}, or {name, error} with error 'blocked' or 'too-big'.
  stageFile(name, data) {
    return this.files.stage(name, data);
  }

  // Files picked in main's dialog, by path (chatfiles.stagePaths).
  stagePaths(paths, room) {
    return this.files.stagePaths(paths, room);
  }

  unstageFile(id) {
    return this.files.unstage(id);
  }

  // Whether a file can go to an agent at all, before main reads it: null, 'blocked' or 'too-big'.
  refuseFile(name, size) {
    return this.files.refuse(name, size);
  }

  openMessageId() {
    const id = this.viewState.openMessageId;
    return id && this.liveMessage(id) ? id : null;
  }

  // What Rukoo has on an email that is there now: its folder cache entry, or for a copy saved on this device the
  // saved list's entry.
  liveMessage(id) {
    if (!id) return null;
    if (String(id).startsWith('saved:')) return this.engine.saved.find((s) => s.id === id) || null;
    return tools.cacheMessage(this.engine, id);
  }

  // The live id of a conversation's email, found like a source card's (locate): ids change when mail moves,
  // the Message-ID header does not, and a copy in another account belongs to another chat. The chat keeps
  // the id it finds. ref: another email the chat keeps, such as one it was continued from.
  async currentMessageId(c, ref = c && c.message) {
    if (!ref) return null;
    const found = await this.locate({ id: ref.id, messageHeader: ref.messageId, accountId: ref.accountId });
    // The chat may have been deleted while a folder synced.
    if (found && found !== ref.id && this.conversations.get(c.id) === c) {
      ref.id = found;
      this.touch(c);
    }
    return found;
  }

  // ---------- model and effort (renderer API) ----------

  // The models and efforts the panel offers for an agent: what the agent itself lists (adapter.models()), and the
  // model in Settings. A list that fails still offers the model in Settings; error says why. refresh: the panel
  // just opened, so a list older than half a minute is loaded again.
  models(agent, { refresh = false } = {}) {
    if (!this.cfg.hasAgent(agent)) throw new AgentError('invalid', 'unknown agent');
    const entry = this.modelLists.get(agent);
    if (entry && entry.pending) return entry.pending;
    if (entry && entry.value && Date.now() - entry.at < (refresh || entry.value.error ? MODELS_RETRY_MS : MODELS_TTL_MS)) return Promise.resolve(entry.value);
    const pending = this.loadModels(agent).then((value) => {
      // Settings changed meanwhile (configChanged() dropped this entry): this list is about the old ones.
      const now = this.modelLists.get(agent);
      if (now && now.pending === pending) this.modelLists.set(agent, { at: Date.now(), value });
      return value;
    });
    // The list before stays in use (effortsFor()) until the new one is in.
    this.modelLists.set(agent, { ...entry, pending });
    return pending;
  }

  async loadModels(agent) {
    const adapter = this.adapters.get(agent);
    const settingsModel = String((this.cfg.agentData(agent) || {}).model || '').trim();
    let res = null;
    let error = null;
    try {
      if (!adapter || typeof adapter.models !== 'function') throw new AgentError('unknown', this.adapterErrors.get(agent) || `${agent} cannot list its models`);
      res = await withTimeout(adapter.models(), 20000);
    } catch (err) {
      error = toError(err);
    }
    const models = [];
    for (const m of res && Array.isArray(res.models) ? res.models : []) {
      if (models.length >= MAX_MODELS) break;
      const id = isObj(m) ? cleanModel(m.id, false) : null;
      if (!id || models.some((x) => x.id === id)) continue;
      models.push({
        id,
        label: clip(m.label || id, 120),
        ...(m.group ? { group: clip(m.group, 80) } : {}),
        ...(m.tag ? { tag: clip(m.tag, 120) } : {}),
        ...(m.route ? { route: true } : {}),
        ...(m.isDefault ? { isDefault: true } : {}),
        efforts: cleanEfforts(m.efforts),
        defaultEffort: EFFORTS.includes(m.defaultEffort) ? m.defaultEffort : ''
      });
    }
    const custom = cleanEfforts(res && res.custom);
    // A model in Settings that the agent does not list is still the default.
    if (settingsModel && !models.some((m) => m.id === settingsModel)) models.unshift({ id: settingsModel, label: settingsModel, efforts: custom, defaultEffort: '' });
    const own = settingsModel ? models.find((m) => m.id === settingsModel) : null;
    return { agent, models, efforts: own ? own.efforts : cleanEfforts(res && res.efforts), custom, defaultModel: settingsModel, error };
  }

  // ---------- the agents' own skills (renderer API) ----------

  // The skills the agent itself has, for the panel's $ menu: {agent, skills: [{name, description, scope}], error}.
  // refresh: the menu just opened, so a list older than ten seconds is loaded again.
  ownSkills(agent, { refresh = false } = {}) {
    if (!this.cfg.hasAgent(agent)) throw new AgentError('invalid', 'unknown agent');
    const entry = this.skillLists.get(agent);
    if (entry && entry.pending) return entry.pending;
    if (entry && entry.value && Date.now() - entry.at < (refresh || entry.value.error ? OWN_SKILLS_RETRY_MS : OWN_SKILLS_TTL_MS)) return Promise.resolve(entry.value);
    const pending = this.loadOwnSkills(agent).then((value) => {
      // Settings changed meanwhile (configChanged() dropped this entry): this list is about the old ones.
      const now = this.skillLists.get(agent);
      if (now && now.pending === pending) this.skillLists.set(agent, { at: Date.now(), value });
      return value;
    });
    this.skillLists.set(agent, { ...entry, pending });
    return pending;
  }

  async loadOwnSkills(agent) {
    const adapter = this.adapters.get(agent);
    try {
      if (!this.cfg.enabled(agent)) throw new AgentError('disabled', `${agent} is turned off`);
      if (!adapter || typeof adapter.skills !== 'function') throw new AgentError('unknown', this.adapterErrors.get(agent) || `${agent} cannot list its skills`);
      return { agent, skills: ownskills.cleanList(await withTimeout(adapter.skills(), 20000)), error: null };
    } catch (err) {
      return { agent, skills: [], error: toError(err) };
    }
  }

  // The agent's own skills that the user's text names as $name, for the adapter to start. Only text the user wrote
  // counts, never what Rukoo adds around it: that can quote an email. The list is the one the menu showed, or a
  // fresh one when there is none; without a list, nothing counts as a skill and the text goes as it is.
  async mentionedSkills(agent, text, action) {
    if (NO_MENTIONS.has(action) || !/(^|\s)\$[a-zA-Z0-9]/.test(String(text || ''))) return [];
    let list;
    try {
      list = await withTimeout(this.ownSkills(agent), 10000);
    } catch (_) {
      return [];
    }
    const names = new Set((list && list.skills ? list.skills : []).map((s) => s.name));
    const seen = new Set();
    return ownskills.mentionsIn(text, names).filter((m) => !seen.has(m.name) && seen.add(m.name)).map((m) => ({ name: m.name }));
  }

  // The efforts the agent's model list offers for a chat's model (null: the default model), or null without a list
  // to go by.
  effortsFor(agent, model) {
    const entry = this.modelLists.get(agent);
    const list = entry && entry.value;
    if (!list || list.error) return null;
    if (!model) return list.efforts;
    const m = list.models.find((x) => x.id === model);
    return m ? m.efforts : list.custom;
  }

  // Applies a model, effort or permission level picked in the panel; null follows the agent's default or, for the
  // level, Settings. An effort the model does not offer goes back to the default. A turn that runs now keeps the model
  // and effort it started with; the level counts from the next tool call, as Rukoo checks it there.
  setChoice(id, choice) {
    const c = this.mustGet(id);
    this.applyChoice(c, choice);
    this.touch(c);
    this.emitEvent({ kind: 'conversation', conversation: { id: c.id, model: c.model, effort: c.effort, permission: c.permission } });
    return { model: c.model, effort: c.effort, permission: c.permission };
  }

  applyChoice(c, choice) {
    if (!isObj(choice)) throw new AgentError('invalid', 'choice must be an object');
    const model = 'model' in choice ? cleanModel(choice.model) : c.model;
    let effort = 'effort' in choice ? choice.effort || null : c.effort;
    if (effort !== null && !EFFORTS.includes(effort)) throw new AgentError('invalid', `effort must be one of ${EFFORTS.join(', ')}`);
    const offered = effort ? this.effortsFor(c.agent, model) : null;
    if (offered && !offered.includes(effort)) effort = null;
    const permission = 'permission' in choice ? choice.permission || null : c.permission || null;
    if (permission !== null && !PERMISSIONS.includes(permission)) throw new AgentError('invalid', `permission must be one of ${PERMISSIONS.join(', ')}`);
    c.model = model;
    c.effort = effort;
    c.permission = permission;
  }

  // The chat's level: its own, else the one in Settings (permission.js). permissionFor() says when a call gets it.
  permissionOf(c) {
    return (c && permissionLevel(c.permission)) || permissionLevel(this.cfg.data.permission) || 'ask';
  }

  // The level a turn tells the agent about. Claude Code's calls carry their chat's token and Codex's its thread, so
  // its own level counts in their chats. Hermes and OpenClaw name the chat in each call themselves, so their chats
  // go no further than Settings (permissionFor()); the panel offers them no looser level either.
  turnPermission(c) {
    const own = this.permissionOf(c);
    return BOUND_AGENTS.has(c.agent) ? own : strictest([own, this.permissionOf(null)]);
  }

  // The effort a turn sends: the chat's, unless the agent's current list no longer offers it for the model.
  turnEffort(c) {
    const offered = c.effort ? this.effortsFor(c.agent, c.model) : null;
    return offered && !offered.includes(c.effort) ? null : c.effort || null;
  }

  // ---------- turns ----------

  // skill: the name of a skill the user started with /name. Rukoo writes that turn itself, from the skill as it is
  // on disk now.
  // files: tokens of files the user attached (stageFile); emails: Rukoo ids of emails the user added. A message
  // with either needs no text.
  // A refusal throws at once, before anything changes. A message with files then returns a promise: the files are
  // copied into the chat's folder without blocking (chatfiles.keep), the chat is busy meanwhile, and the message
  // shows in the chat only once they are there. A failed copy rejects it and leaves the chat as it was.
  // mode: what happens to a message sent while the chat is busy (hold()). 'steer' hands it to the agent before its
  // next step, 'queue' sends it after the turn. Without one, a busy chat refuses the message.
  send(id, { text, action = null, display = null, skill = null, files = [], emails = [], mode = null } = {}) {
    const c = this.mustGet(id);
    if (this.disposed) throw new AgentError('stopped', 'Rukoo is closing');
    const busy = this.turns.has(c.id) || this.sending.has(c.id);
    if (busy && mode !== 'steer' && mode !== 'queue') throw new AgentError('busy', 'a turn is running');
    if (!this.cfg.enabled(c.agent)) throw new AgentError('disabled', `${c.agent} is turned off`);
    if (skill) {
      const s = this.skills.get(skill);
      if (!s) throw new AgentError('skill-missing', `no skill named ${clip(skill, 80)}`);
      text = context.skillTurn(s);
      action = 'skill';
      display = `/${s.name}`;
    }
    const body = clip(text, 20000);
    // Checked before anything changes, so a refusal leaves the chat and the user's attachments as they were.
    const fileIds = [].concat(files || []).map(String);
    if (fileIds.length > MAX_FILES) throw new AgentError('invalid', `at most ${MAX_FILES} files per message`);
    const staged = this.files.pending(fileIds);
    if (!staged) throw missingError('file-missing', this.files.missing(fileIds), 'a file is no longer ready to send; attach it again');
    const mails = this.attachEmails(c, emails);
    if (!body.trim() && !staged.length && !mails.length) throw new AgentError('invalid', 'empty message');
    const shown = display ? clip(display, 20000) : body;
    // display can be an approved proposal's title. Email text the agent quoted in it keeps its <unsafe_content>
    // tags for the agent (runTurn gets them); the user's line and the chat title show the text alone.
    const line = display ? context.untag(shown) : shown;
    const message = { body, action, shown, line, mails };
    if (busy) return this.hold(c, message, staged, mode);
    if (!staged.length) return this.begin(c, message, []);
    // Removed while its files were copied: what was copied goes with the chat's folder, whichever way the copy went.
    const removed = () => {
      if (this.conversations.get(c.id) === c) return false;
      this.files.remove(c.id);
      return true;
    };
    const sent = this.files.keep(c.id, staged).then(
      async (kept) => {
        try {
          if (removed()) throw new AgentError('invalid', 'unknown conversation');
          // Closed meanwhile: the chat keeps its earlier files only. dispose() waits for this.
          if (this.disposed) {
            await this.files.discard(c.id, kept);
            throw new AgentError('stopped', 'Rukoo is closing');
          }
        } finally {
          this.sending.delete(c.id);
        }
        return this.begin(c, message, kept);
      },
      (err) => {
        this.sending.delete(c.id);
        if (removed()) throw new AgentError('invalid', 'unknown conversation');
        // keep() stopped copying once Rukoo began to close, and removed what it had copied.
        if (this.disposed) throw new AgentError('stopped', 'Rukoo is closing');
        this.nextFollowUp(c.id);
        throw err;
      }
    );
    this.sending.set(c.id, sent);
    return sent;
  }

  // The rest of send(), once the message's files are in the chat's folder (kept): the message goes into the chat,
  // and its turn starts.
  begin(c, { body, action, shown, line, mails }, kept) {
    const firstTurn = !c.delivered;
    // The agent's new session has not had the chat yet: the email and a recap go along again.
    const recap = !firstTurn && c.needsRecap === true;
    // A message that steered before (its files went to the chat then) and came back to the queue brings them again.
    this.addFiles(c, kept);
    // A chat is named after its first message, or after what it brought when it has no text.
    const naming = !c.title;
    if (naming) c.title = clip((line.trim() ? line : [...kept.map((f) => f.name), ...mails.map((m) => m.subject)].join(', ')).replace(/\s+/g, ' ').trim(), 80);
    const userItem = this.addItem(c, {
      type: 'user',
      text: line,
      action: action || null,
      ...(kept.length ? { files: kept.map(shownFile) } : {}),
      ...(mails.length ? { emails: mails.map(({ id: mailId, subject, from }) => ({ id: mailId, subject, from })) } : {})
    });
    // The renderer names a chat after its first message's text by itself; a name taken from attachments only main
    // has, and it reaches the renderer here.
    if (naming) this.emitEvent({ kind: 'conversation', conversation: { id: c.id, title: c.title } });
    const turn = {
      id: `t_${rand(10)}`,
      cid: c.id,
      // Where this turn starts in the transcript; a recap covers what came before it.
      userItem: userItem.id,
      controller: new AbortController(),
      items: new Set(),
      tools: new Map(),
      assistant: null,
      thinking: null,
      closed: false,
      errorShown: false,
      // Set once the agent shows it got the input: it streamed, called a tool or asked for approval.
      reached: false,
      // The model and effort as they were when the user sent this: a change while the turn prepares (the email
      // loads) is for the next turn.
      model: c.model || null,
      effort: this.turnEffort(c),
      stopTimer: null,
      forceStop: null,
      // What the user attached to this message; runTurn adds the start of each file's text.
      attached: kept.length || mails.length ? { files: kept, emails: mails } : null,
      // What runTurn hands the adapter, once it does; ready settles then, or when the turn ends before (steer()).
      handle: null,
      ready: null,
      markReady: null
    };
    turn.ready = new Promise((resolve) => {
      turn.markReady = resolve;
    });
    this.turns.set(c.id, turn);
    this.setStatus(c, 'running');
    this.runTurn(c, turn, { text: body, action, firstTurn, recap, display: shown }).catch((err) => this.finishTurn(turn, { status: 'error', error: toError(err) }));
    return { conversationId: c.id, itemId: userItem.id };
  }

  // Emails the user added to a message, by Rukoo id. Only an email Rukoo has itself counts: one in the user's
  // accounts, or a copy saved on this device. Subject and sender come from Rukoo's own cache, not the renderer.
  // The chat's own email is there already. Each keeps its Message-ID and account, as the chat's own email does:
  // a Rukoo id changes when the email moves, and with those Rukoo finds it again (refreshEmails(), relocate()).
  attachEmails(c, ids) {
    const list = [].concat(ids || []);
    if (list.length > MAX_EMAILS) throw new AgentError('invalid', `at most ${MAX_EMAILS} emails per message`);
    const out = [];
    const gone = [];
    // One email can have several Rukoo ids (Inbox and Sent of a mail to yourself, two Gmail labels): by its
    // Message-ID it goes once, and not when it is the chat's own.
    const same = (a, b) => Boolean(a && b) && normId(a) === normId(b);
    const seen = [];
    for (const raw of list) {
      const id = clip(raw, 4000);
      if (!id || out.some((m) => m.id === id) || gone.includes(id) || (c.message && c.message.id === id)) continue;
      const m = this.liveMessage(id);
      if (!m) {
        gone.push(id);
        continue;
      }
      if (seen.some((key) => same(key, m.messageId)) || same(c.message && c.message.messageId, m.messageId)) continue;
      // A newer email the chat was continued from is in it already: the next turn carries it by itself while the
      // agent has not had it (loadNewer()), so attached as well it would go twice and take a place.
      if ((c.continued || []).some((e) => e.id === id || same(e.messageId, m.messageId))) continue;
      if (m.messageId) seen.push(m.messageId);
      const from = isObj(m.from) ? { name: clip(m.from.name || '', 200), address: clip(m.from.address || '', 320) } : null;
      // A saved copy's id names no account; its entry does.
      const accountId = id.startsWith('saved:') ? m.accountId || null : tools.accountOfId(id);
      out.push({ id, subject: clip(m.subject || '', 300), from, messageId: m.messageId ? clip(m.messageId, 1000) : null, accountId });
    }
    if (gone.length) throw missingError('email-missing', gone, 'an email added to the message is no longer here');
    return out;
  }

  // Hermes usually runs on another machine and reaches Rukoo through its bridge, so a path on this PC means nothing
  // to it, and its Runs API takes text only. An OpenClaw gateway can be elsewhere too, and Rukoo sends it text only.
  // Claude Code and Codex run here. A custom agent's server can be anywhere, so it gets text only too.
  localAgent(c) {
    return c.agent !== 'clark' && c.agent !== 'openclaw' && !isCustomId(c.agent);
  }

  async runTurn(c, turn, { text, action, firstTurn, recap = false, display = '' }) {
    const adapter = this.adapters.get(c.agent);
    let result;
    if (action === 'approved') turn.approval = { display: display || text };
    // Before the first await, so the message in the transcript has each email's Message-ID from the moment it is sent.
    this.keepEmails(c, turn);
    try {
      if (!adapter) throw new AgentError('unknown', this.adapterErrors.get(c.agent) || `${c.agent} adapter not available`);
      // Mail work that still runs reports into the notes this turn takes: wait for all of it first, also for
      // what is decided while this waits.
      if (this.mailSettling.has(c.id)) {
        for (let work = this.mailSettling.get(c.id); work; work = this.mailSettling.get(c.id)) await work;
        if (turn.closed || turn.controller.signal.aborted) {
          this.giveBack(c, turn);
          return this.finishTurn(turn, { status: 'stopped' });
        }
      }
      let message = null;
      if ((firstTurn || recap) && c.message) {
        message = await this.loadEmail(c);
        // Stopped while the email was loading: stop() already closed the turn, and a newer turn may own the
        // notes by now. Touch nothing.
        if (turn.closed || turn.controller.signal.aborted) {
          // An approved follow-up stopped this early never reached the agent; keep the approval (the notes
          // are not taken yet, so nothing else changes).
          this.giveBack(c, turn);
          return this.finishTurn(turn, { status: 'stopped' });
        }
      }
      // Newer emails in the thread the user continued this chat from go along once. A new session has had none of
      // the chat, so it gets the newest one again.
      let newer = [];
      if ((c.continued || []).length) {
        newer = await this.loadNewer(c, { again: recap });
        // Stopped while it loaded, as above.
        if (turn.closed || turn.controller.signal.aborted) {
          this.giveBack(c, turn);
          return this.finishTurn(turn, { status: 'stopped' });
        }
      }
      turn.newer = newer;
      // The attached files from the chat's folder, with the start of their text. A PDF's text comes from a process
      // of its own and can take a while; stopped meanwhile, as above.
      if (turn.attached && turn.attached.files.length) {
        turn.attached = { files: await this.files.forTurn(c.id, turn.attached.files, { signal: turn.controller.signal }), emails: turn.attached.emails };
        if (turn.closed || turn.controller.signal.aborted) {
          this.giveBack(c, turn);
          return this.finishTurn(turn, { status: 'stopped' });
        }
      }
      // An email the user attached can have moved since Send, and one attached to an earlier message since then: the
      // agent gets the ids they have now. Finding a moved one can open a folder; stopped meanwhile, as above.
      const emails = [...(turn.attached ? turn.attached.emails : []), ...(recap ? this.earlierEmails(c, turn) : [])];
      if (emails.length) {
        await this.refreshEmails(c, emails);
        if (turn.closed || turn.controller.signal.aborted) {
          this.giveBack(c, turn);
          return this.finishTurn(turn, { status: 'stopped' });
        }
      }
      // The agent's own skills the user named ($name). Hermes can take a moment to list them; stopped meanwhile, as
      // above.
      const named = await this.mentionedSkills(c.agent, text, action);
      if (turn.closed || turn.controller.signal.aborted) {
        this.giveBack(c, turn);
        return this.finishTurn(turn, { status: 'stopped' });
      }
      const openMessage = firstTurn ? null : this.openLine(c, newer);
      const notes = c.notes.splice(0);
      turn.notes = notes;
      const local = this.localAgent(c);
      // The instructions describe Ask. The turn names another level, and Ask again after the agent heard of another.
      turn.permission = this.turnPermission(c);
      const told = firstTurn || recap ? null : c.toldPermission;
      const permission = turn.permission !== 'ask' || (told && told !== 'ask') ? turn.permission : null;
      const input = context.turnText({ conversation: c, text, firstTurn, notes, message, openMessage, newer, earlier: recap ? this.earlier(c, turn) : null, attached: turn.attached, local, permission, meeting: this.meetingTurn(c) });
      const port = this.mcp ? this.mcp.ports().local : null;
      const attached = turn.attached || { files: [], emails: [] };
      const handle = {
        id: turn.id,
        conversation: c,
        input,
        instructions: context.instructions({ agent: c.agent, agentName: this.agentName(c.agent) }),
        text,
        action: action || null,
        // The agent's own skills the user's text names as $name ([{name}]), in that order. The adapter starts them
        // the way its agent does; text keeps the mentions as the user wrote them.
        skills: named,
        firstTurn,
        // The chat's own model and effort; null: the adapter uses the model in Settings and the agent's own effort.
        model: turn.model,
        effort: turn.effort,
        // What the user attached. Adapters that take images send each inline one along as an image.
        files: attached.files.filter((f) => !f.missing).map(({ id, name, type, kind, size, path: file, inline }) => ({ id, name, type, kind, size, path: file, inline: Boolean(inline) })),
        emails: attached.emails.map((e) => e.id),
        mcp: { url: port ? `http://127.0.0.1:${port}/mcp` : '', token: this.tokenFor(c) },
        emit: (event) => this.onAdapterEvent(turn, event),
        approve: (request) => this.requestApproval(c.id, { ...(request || {}), source: c.agent, kind: 'runtime' }),
        setProvider: (patch) => {
          if (!isObj(patch)) return;
          c.provider = { ...c.provider, ...patch };
          if (patch.threadId) this.mapThread(patch.threadId, c.id);
          this.touch(c);
        },
        // Optional for adapters: call once the agent has the input, a more exact signal than its first output.
        accepted: () => {
          turn.reached = true;
        },
        // Optional for adapters: the agent no longer has this chat's session, and the adapter starts a new one.
        // Await it before sending input, which then carries the email and a recap of the chat.
        startOver: () => this.startOver(c, turn, handle, { text, ready: firstTurn || recap }),
        // For adapters that steer (steer()): the agent took in the message with this id, as part of this turn.
        steered: (id) => this.onSteered(c, turn, id),
        // Whether the hub still follows this turn: a steer that waits for the agent to be ready stops waiting.
        isOpen: () => !turn.closed,
        signal: turn.controller.signal
      };
      const forced = new Promise((resolve) => {
        turn.forceStop = resolve;
      });
      // Never start the agent for a turn that is already stopped.
      if (turn.controller.signal.aborted) result = { status: 'stopped' };
      else {
        turn.handle = handle;
        turn.markReady(true);
        result = await Promise.race([adapter.runTurn(handle), forced]);
      }
    } catch (err) {
      result = { status: 'error', error: toError(err) };
    }
    // A turn that never reached the agent (offline, not installed, stopped early) uses up nothing: the next
    // message is still a first turn with the email, and the notes go along with it.
    if (turn.reached || (result && result.status === 'done')) {
      if (!c.delivered || c.needsRecap) {
        c.delivered = true;
        c.needsRecap = false;
        this.touch(c);
      }
      if (turn.permission && c.toldPermission !== turn.permission) {
        c.toldPermission = turn.permission;
        this.touch(c);
      }
      // The agent has the newer emails this turn carried. Any it did not carry still wait (see loadNewer()).
      const told = (turn.newer || []).filter((n) => n.ref.pending);
      if (told.length) {
        for (const n of told) n.ref.pending = false;
        c.continued = this.pruneContinued(c.continued || []);
        this.touch(c);
        this.emitContinued(c);
      }
    } else this.giveBack(c, turn);
    this.finishTurn(turn, result || { status: 'error', error: { code: 'protocol', detail: 'the adapter returned nothing' } });
  }

  // The chat's own email in full, found again if it moved; null when it is gone or does not load. Never the email
  // that happens to be open now. ref: another email the chat keeps (currentMessageId).
  async loadEmail(c, ref = c.message) {
    const liveId = await this.currentMessageId(c, ref);
    const full = liveId ? await this.engine.getMessage(liveId).catch(() => null) : null;
    if (!full) return null;
    // Chats saved before Rukoo kept these get them here, with their first message after an update.
    if (this.keepThread(ref, full) && this.conversations.get(c.id) === c) this.touch(c);
    const acc = this.engine.accounts.find((a) => a.id === full.accountId);
    return { full, account: acc ? acc.email : '', folder: full.folder || '' };
  }

  // The newer emails the user continued this chat from (continueFrom()) that the agent has not had yet, by the emails'
  // dates, oldest first: the newest MAX_NEWER of them, so one message stays a sensible size. Older ones wait for the
  // next message. With again set and none waiting, the newest one the agent had already: a new session has had
  // nothing. Each is {ref, full, account, folder, open}, with full null when it does not load; the turn names that
  // one instead.
  async loadNewer(c, { again = false } = {}) {
    const byDate = (list) => list.slice().sort((a, b) => (Number(a.date) || 0) - (Number(b.date) || 0));
    const all = c.continued || [];
    let refs = byDate(all.filter((e) => e.pending)).slice(-MAX_NEWER);
    if (!refs.length && again) refs = byDate(all).slice(-1);
    const out = [];
    for (const ref of refs) {
      let email = null;
      try {
        email = await this.loadEmail(c, ref);
      } catch (err) {
        this.log('hub', 'loading the newer email failed:', err && err.message);
      }
      out.push({ ref, full: email ? email.full : null, account: email ? email.account : '', folder: email ? email.folder : '' });
    }
    // After the loads: finding a moved email gives it its current id.
    const open = this.openMessageId();
    for (const n of out) n.open = Boolean(open && open === n.ref.id);
    return out;
  }

  // The email the user looks at now, as {id, subject}, when it is not the chat's own. null as well when it is the one
  // newer email this turn carries: that one's own line says the user looks at it.
  openLine(c, newer) {
    const openId = this.openMessageId();
    const saysSo = Array.isArray(newer) && newer.length === 1 && newer[0].open;
    if (!openId || openId === (c.message ? c.message.id : null) || saysSo) return null;
    const m = this.liveMessage(openId);
    return { id: openId, subject: m ? m.subject : '' };
  }

  // The transcript before this turn's own message.
  earlier(c, turn) {
    const at = c.items.findIndex((i) => i.id === turn.userItem);
    return at < 0 ? c.items.slice() : c.items.slice(0, at);
  }

  // The emails the user attached to the messages before this turn's own, as their messages keep them.
  earlierEmails(c, turn) {
    return this.earlier(c, turn).flatMap((i) => (i.type === 'user' && Array.isArray(i.emails) ? i.emails : []));
  }

  // The user's message keeps the emails it took along as this turn has them, with their Message-ID and account
  // (attachEmails()), so a recap and read_message find one again after it moved. One list for both: an id the turn
  // finds again (refreshEmails()) is the message's as well. Chats saved before Rukoo kept these have the id alone.
  keepEmails(c, turn) {
    const emails = turn.attached ? turn.attached.emails : [];
    const item = emails.length ? c.items.find((i) => i.id === turn.userItem) : null;
    if (item) item.emails = emails;
  }

  // Attached emails where they are now, found like the chat's own (currentMessageId): each one that moved gets
  // its new id, in the transcript too. One without a Message-ID, or one that is gone, keeps the id it has. One at a
  // time: finding a moved email can open its folder.
  async refreshEmails(c, emails) {
    for (const e of emails) {
      if (isObj(e) && e.id && e.messageId) await this.currentMessageId(c, e);
    }
  }

  // The current id of an email this chat gave the agent, by an id that no longer holds it: the chat's own, one it
  // was continued from, or one the user attached. Each keeps its Message-ID and account, so one that moved is found
  // again (locate). Nothing in the chat changes, so the agent can pass the same id again; a recap gives a new
  // session the new ids (refreshEmails()). null when the chat gave no email that id, or the email is gone.
  async relocate(c, id) {
    if (!c || !id || this.conversations.get(c.id) !== c) return null;
    const attached = c.items.flatMap((i) => (i.type === 'user' && Array.isArray(i.emails) ? i.emails : []));
    const ref = [c.message, ...(c.continued || []), ...attached].find((e) => isObj(e) && e.id === id && e.messageId);
    return ref ? this.locate({ id, messageHeader: ref.messageId, accountId: ref.accountId || null }) : null;
  }

  // The agent lost the chat's session (Claude Code deletes old transcripts, say), and the adapter starts a new
  // one that knows nothing of the chat. Before the adapter sends anything, input becomes a first turn with the
  // chat's email and a recap of the chat. A turn that has the email already (ready) stays as it is: a first turn
  // has nothing to recap. Never throws, so a lost session costs the adapter one await.
  async startOver(c, turn, handle, { text, ready }) {
    if (ready || turn.startedOver) return handle.input;
    turn.startedOver = true;
    // Kept until a turn reaches the new session, so a turn that fails on the way does not leave it without the email.
    c.needsRecap = true;
    // The new session has read none of the meeting's transcript either: its next read starts from the beginning.
    if (c.meeting) {
      c.meeting.readSeq = 0;
      delete c.meeting.paging;
    }
    this.touch(c);
    let message = null;
    try {
      message = c.message ? await this.loadEmail(c) : null;
    } catch (err) {
      this.log('hub', 'loading the email for a new session failed:', err && err.message);
    }
    // The new session has not had a newer email either, even if the old one did.
    const newer = turn.newer && turn.newer.length ? turn.newer : (c.continued || []).length ? await this.loadNewer(c, { again: true }) : [];
    // The recap names the emails of earlier messages by the ids they have now.
    await this.refreshEmails(c, this.earlierEmails(c, turn));
    // Stopped meanwhile: the adapter sees the abort and sends nothing.
    if (turn.closed || turn.controller.signal.aborted) return handle.input;
    turn.newer = newer;
    const openMessage = this.openLine(c, newer);
    // A new session knows only the instructions, which describe Ask: it hears of any other level here.
    const permission = turn.permission && turn.permission !== 'ask' ? turn.permission : null;
    handle.input = context.turnText({ conversation: c, text, firstTurn: true, notes: turn.notes, message, openMessage, newer, earlier: this.earlier(c, turn), attached: turn.attached, local: this.localAgent(c), permission, meeting: this.meetingTurn(c) });
    return handle.input;
  }

  // What a meeting chat's turn says about its meeting (context.turnText): the title and state, and how many passages
  // changed since the agent last read the transcript. off: the user turned agents' access to meetings off, so the
  // turn says nothing from the meeting. known: Rukoo has the meeting. null for any other chat.
  meetingTurn(c) {
    if (!c.meeting) return null;
    const off = this.meetingsOff();
    const voice = this.deps.voice;
    const m = !off && voice ? voice.meeting(c.meeting.id) : null;
    return {
      id: c.meeting.id,
      off,
      known: Boolean(m),
      title: m ? m.meeting.title : c.meeting.title,
      state: m ? m.meeting.state : null,
      fresh: m ? m.passages.filter((p) => p.changed > c.meeting.readSeq).length : 0,
      total: m ? m.passages.length : 0
    };
  }

  // The user turned off agents' access to meetings (Settings > Meetings > Agents can read meetings).
  meetingsOff() {
    return Boolean(this.engine && this.engine.settings && this.engine.settings.meetingAgentsCanRead === false);
  }

  tokenFor(c) {
    let token = this.convTokens.get(c.id);
    if (!token) {
      token = this.issueToken({ agent: c.agent, conversationId: c.id });
      this.convTokens.set(c.id, token);
    }
    return token;
  }

  onAdapterEvent(turn, event) {
    if (turn.closed || !isObj(event)) return;
    const c = this.conversations.get(turn.cid);
    if (!c) return;
    if (REACHED.has(event.type)) turn.reached = true;
    const now = Date.now();
    const own = (fields) => {
      const item = this.addItem(c, fields);
      turn.items.add(item.id);
      return item;
    };
    // Text after any other item starts a new bubble; anything that is not thinking ends the thinking state.
    const endThinking = () => {
      if (turn.thinking && turn.thinking.status === 'running') {
        Object.assign(turn.thinking, { status: 'done', endedAt: now });
        this.updateItem(c, turn.thinking);
      }
      turn.thinking = null;
    };
    const endText = () => {
      if (turn.assistant && turn.assistant.status === 'streaming') {
        turn.assistant.status = 'done';
        this.updateItem(c, turn.assistant);
      }
      turn.assistant = null;
    };
    switch (event.type) {
      case 'text': {
        const delta = String(event.delta || '');
        if (!delta) return;
        endThinking();
        if (!turn.assistant) {
          turn.assistant = own({ type: 'assistant', text: delta.slice(0, MAX_TEXT), status: 'streaming', interim: false });
          return;
        }
        const room = MAX_TEXT - turn.assistant.text.length;
        if (room <= 0) return;
        const piece = delta.slice(0, room);
        turn.assistant.text += piece;
        // No save per delta: the next item, the end of the turn or quitting saves the text.
        c.updatedAt = now;
        this.queueDelta(c.id, turn.assistant.id, piece);
        return;
      }
      case 'commentary': {
        const text = String(event.text || '').trim();
        if (!text) return;
        endThinking();
        endText();
        own({ type: 'assistant', text, status: 'done', interim: true });
        return;
      }
      case 'thinking': {
        const delta = String(event.delta || '');
        endText();
        if (!turn.thinking) {
          turn.thinking = own({ type: 'thinking', text: delta.slice(0, MAX_TEXT), status: 'running', startedAt: now, endedAt: null });
          return;
        }
        if (!delta) return;
        const piece = delta.slice(0, Math.max(0, MAX_TEXT - turn.thinking.text.length));
        if (!piece) return;
        turn.thinking.text += piece;
        this.queueDelta(c.id, turn.thinking.id, piece);
        return;
      }
      case 'thinking-done':
        endThinking();
        return;
      case 'tool-start': {
        endThinking();
        endText();
        const name = clip(event.name || 'tool', 200);
        const key = String(event.key || `${name}#${turn.tools.size}`);
        const item = own({
          type: 'tool',
          name,
          label: toolLabel(name),
          detail: chipDetail(event.detail),
          status: 'running',
          startedAt: now,
          endedAt: null,
          rukoo: tools.isRukooTool(name)
        });
        turn.tools.set(key, item);
        return;
      }
      case 'tool-end': {
        const item = turn.tools.get(String(event.key || ''));
        if (!item || item.status !== 'running') return;
        item.status = event.error ? 'error' : 'done';
        item.endedAt = now;
        if (event.detail) item.detail = chipDetail(event.detail);
        this.updateItem(c, item);
        return;
      }
      case 'error': {
        endThinking();
        endText();
        const err = toError(event);
        turn.errorShown = true;
        turn.lastError = err;
        own({ type: 'notice', text: ERROR_TEXT[err.code], detail: err.detail, tone: 'error', code: err.code, undo: null });
        return;
      }
      case 'notice': {
        // Informational lines from an adapter, e.g. "Codex started a new thread".
        const text = clip(oneLine(event.text), 500);
        if (!text) return;
        endThinking();
        endText();
        own({ type: 'notice', text, tone: ['info', 'success', 'error'].includes(event.tone) ? event.tone : 'info', code: event.code || null, undo: null });
        return;
      }
      default:
        // 'usage' and anything newer: ignored for now.
        return;
    }
  }

  // Stop ends the turn, not what the user already approved: a queued approval follow-up still starts once
  // the turn has ended (SPEC 4.4 step 6).
  stop(id) {
    const turn = this.turns.get(id);
    if (!turn) return false;
    turn.controller.abort();
    this.expireTurnApprovals(turn);
    // Still loading the email or reading the PDFs: no adapter runs yet, so there is nothing to wait for. What the
    // message brought goes back now, before another message can be sent, not once runTurn() gets past its await.
    if (!turn.forceStop) {
      const c = this.conversations.get(turn.cid);
      if (c && !turn.reached) this.giveBack(c, turn);
      this.finishTurn(turn, { status: 'stopped' });
      return true;
    }
    if (!turn.stopTimer) turn.stopTimer = setTimeout(() => turn.forceStop && turn.forceStop({ status: 'stopped' }), STOP_GRACE_MS);
    return true;
  }

  expireTurnApprovals(turn) {
    for (const [itemId, w] of this.approvals) {
      if (w.turnId !== turn.id) continue;
      this.expireApproval(w.cid, itemId);
    }
  }

  finishTurn(turn, result) {
    if (turn.closed) return;
    turn.closed = true;
    clearTimeout(turn.stopTimer);
    // Steers still waiting for the agent to be ready stop waiting (deliverSteer()).
    if (turn.markReady) turn.markReady(false);
    if (this.turns.get(turn.cid) === turn) this.turns.delete(turn.cid);
    const c = this.conversations.get(turn.cid);
    const status = ['done', 'stopped', 'error'].includes(result && result.status) ? result.status : 'error';
    this.expireTurnApprovals(turn);
    if (!c) return;
    this.lastOutcome.set(c.id, status);
    // A message that steered but that the agent never took in (the turn ended first, or was stopped) goes back to the
    // queue, ahead of the ones that waited for the turn to end: the user wanted it sooner. So does one that was to
    // steer once its files were in (hold()), in the order sent.
    const back = c.queue.filter((e) => e.state === 'steering' || (e.state === 'copying' && turn.copyingSteers && turn.copyingSteers.has(e)));
    if (back.length) {
      for (const e of back) if (e.state === 'steering') e.state = 'queued';
      c.queue = [...back, ...c.queue.filter((e) => !back.includes(e))];
      this.touch(c);
      this.emitQueue(c);
    }
    const now = Date.now();
    for (const item of c.items) {
      if (!turn.items.has(item.id)) continue;
      let changed = false;
      if (item.type === 'assistant' && item.status === 'streaming') {
        item.status = status === 'stopped' ? 'stopped' : 'done';
        changed = true;
      } else if ((item.type === 'thinking' || item.type === 'tool') && item.status === 'running') {
        Object.assign(item, { status: 'done', endedAt: now });
        changed = true;
      }
      if (changed) this.updateItem(c, item);
    }
    let error = null;
    if (status === 'error') {
      error = (result && result.error && toError(result.error)) || turn.lastError || { code: 'unknown', detail: '' };
      if (!turn.errorShown) this.addItem(c, { type: 'notice', text: ERROR_TEXT[error.code], detail: error.detail, tone: 'error', code: error.code, undo: null });
    }
    this.setStatus(c, status === 'error' ? 'error' : 'idle', error);
    // An answer or an error in a chat that is not on screen waits in the panel's chat list. Stop was the user's own doing.
    if (status !== 'stopped' && this.watching !== c.id) this.markUnread(c, status === 'error' ? 'error' : 'reply');
    if (error && ['offline', 'unauthorized', 'not-configured', 'not-installed', 'spawn-failed'].includes(error.code)) {
      this.refreshStatus().catch(() => {});
    }
    // After a stop or an error the waiting messages stay where they are, until the user sends one or another message:
    // Stop means stop, and after an error the next message would likely meet the same problem.
    this.nextFollowUp(c.id, { queue: status === 'done' });
  }

  // An approval that waited for the chat to be free (onProposal) goes now: its turn ended, or a message's files
  // failed to copy. Without one, and with queue set, the next message the user queued goes (nextQueued()).
  nextFollowUp(cid, { queue: andQueue = false } = {}) {
    if (this.disposed) return;
    const queue = this.followUps.get(cid);
    const next = queue && queue.shift();
    if (queue && !queue.length) this.followUps.delete(cid);
    if (next) setImmediate(() => this.sendFollowUp(cid, next));
    else if (andQueue) setImmediate(() => this.nextQueued(cid));
  }

  sendFollowUp(cid, message) {
    if (this.disposed || !this.conversations.has(cid)) return;
    try {
      this.send(cid, message);
    } catch (err) {
      const c = this.conversations.get(cid);
      const e = toError(err);
      if (!c) return;
      this.addItem(c, { type: 'notice', text: ERROR_TEXT[e.code] || ERROR_TEXT.unknown, detail: e.detail, tone: 'error', code: e.code, undo: null });
      if (message.action === 'approved') this.keepApproval(c, message);
    }
  }

  // A turn that never reached the agent (offline, failed to start, stopped first) uses up nothing: the notes it
  // took go back, and an approved follow-up's approval is kept, so the next message still tells the agent what
  // the user said yes to. Once per turn, from runTurn() or from dispose(), whichever comes first.
  giveBack(c, turn) {
    if (turn.givenBack) return;
    turn.givenBack = true;
    if (turn.notes && turn.notes.length) {
      c.notes.unshift(...turn.notes);
      this.touch(c);
    }
    if (turn.approval) this.keepApproval(c, turn.approval);
    // The chat keeps the files; the next message says what the user attached and how to read it.
    const a = turn.attached;
    if (a && (a.files.length || a.emails.length)) {
      const names = [
        ...a.files.map((f) => `${context.unsafeInline(f.name, 'file name', 100)} (file_id ${f.id})`),
        ...a.emails.map((e) => `the email ${context.unsafeInline(e.subject || '', 'email subject', 100)} (id ${e.id})`)
      ];
      c.notes.push(`The user attached ${names.join(', ')} to a message that did not reach you. read_chat_file and read_message return them.`);
      this.touch(c);
    }
  }

  // An approval whose follow-up turn could not start. The agent hears about it with the next message, and
  // the card's "Approved" does not pretend it was carried out.
  keepApproval(c, message) {
    c.notes.push(`The user approved: ${message.display}. You were not told until now, so it has not been done; check with the user before you carry it out.`);
    const title = context.untag(message.display);
    this.addItem(c, { type: 'notice', text: `Not done yet: ${title}. ${this.agentName(c.agent)} hears about your approval with your next message.`, tone: 'info', code: 'kept-approval', params: { title: title.slice(0, 200) }, undo: null });
  }

  // ---------- messages sent while the agent works ----------
  //
  // A message the user sends while the chat is busy waits in c.queue, oldest first, as {id, state, at, text (what
  // the user sees), body (what the agent gets), shown, action, files, emails, fallback}. 'queued' waits for the turn
  // to end; 'steering' went to the agent, which takes it in before its next step, and then it joins the transcript
  // (onSteered()). After a turn that ended normally the next queued message goes as a turn of its own. After Stop,
  // an error or a restart they wait until the user sends one of them, or another message. 'copying' keeps the place
  // of a message whose files are still being copied (hold()): the panel doesn't see it yet, and the messages behind
  // it wait for it.
  //
  // An adapter that can take a message during a turn has steer(turn, {id, input, text, files}), which resolves true
  // once the agent will get it in this turn, and calls turn.steered(id) when the agent takes it in. canSteer(), when
  // it has one, says whether it can right now (a Hermes server without the steer route can't).

  canSteer(agent) {
    const a = this.adapters.get(agent);
    if (!a || typeof a.steer !== 'function') return false;
    return typeof a.canSteer === 'function' ? a.canSteer() !== false : true;
  }

  emitQueue(c) {
    this.emitEvent({ kind: 'conversation', conversation: { id: c.id, queue: shownQueue(c.queue) } });
  }

  // The chat as the panel gets it (agentGet): without the messages whose files are still being copied.
  forPanel(id) {
    const c = this.get(id);
    return c && c.queue.some((e) => e.state === 'copying') ? { ...c, queue: shownQueue(c.queue) } : c;
  }

  // Files the chat keeps, each once.
  addFiles(c, kept) {
    const have = new Set((c.files || []).map((f) => f.id));
    const add = (kept || []).filter((f) => !have.has(f.id));
    if (add.length) c.files = [...(c.files || []), ...add];
  }

  // A message sent while the chat is busy (send() with a mode). 'steer' hands it to the agent now (steerEntry());
  // anything else waits for the turn to end. Quick actions and skills always wait: each starts a task of its own.
  // Its files go into the chat's folder at once, so a message that waits through a restart keeps them; as with
  // send(), the promise settles once they are there. It takes its place in the queue when the user sends it, so a
  // message sent after it, without files, can't get ahead of it while its files are copied.
  hold(c, message, staged, mode) {
    if (c.queue.length >= MAX_QUEUE) throw new AgentError('invalid', `at most ${MAX_QUEUE} messages can wait`);
    const startsSkill = !message.action && this.namesOwnSkill(c.agent, message.body);
    const steer = mode === 'steer' && !message.action && !startsSkill;
    const entry = {
      id: newQueueId(),
      state: staged.length ? 'copying' : 'queued',
      at: Date.now(),
      text: message.line,
      body: message.body,
      shown: message.shown,
      action: message.action || null,
      files: [],
      emails: message.mails,
      // Set when it was meant to steer but the agent can't take it during this turn.
      fallback: false,
      // It names one of the agent's own skills, so it never steers (namesOwnSkill()).
      startsSkill
    };
    c.queue.push(entry);
    this.touch(c);
    // The steers of a turn go one by one in the order sent (steerEntry()); one whose files are being copied keeps its
    // turn among them.
    let slot = null;
    const ready = (kept) => {
      entry.files = kept;
      entry.state = 'queued';
      this.touch(c);
      // Its turn among the steers is still to come: it goes then (deliverSteer()).
      if (slot && slot.turn === this.turns.get(c.id) && !slot.turn.closed) {
        entry.state = 'steering';
        this.emitQueue(c);
      } else if (steer) this.steerEntry(c, entry);
      else this.emitQueue(c);
      const result = { conversationId: c.id, queued: entry.id, state: entry.state };
      // The turn ended while the files were copied. After a normal end the queue moves on, this message included;
      // after Stop or an error it waits, like the rest.
      this.moveOn(c);
      return result;
    };
    if (!staged.length) return ready([]);
    const done = this.files.keep(c.id, staged).then(
      async (kept) => {
        if (this.conversations.get(c.id) !== c) {
          this.files.remove(c.id);
          throw new AgentError('invalid', 'unknown conversation');
        }
        if (this.disposed) {
          this.dropEntry(c, entry);
          await this.files.discard(c.id, kept);
          throw new AgentError('stopped', 'Rukoo is closing');
        }
        return ready(kept);
      },
      (err) => {
        // Not sent after all: the messages it held back go on.
        this.dropEntry(c, entry);
        this.moveOn(c);
        throw err;
      }
    );
    if (steer) slot = this.steerEntry(c, entry, done);
    return done;
  }

  // Whether a message names one of the agent's own skills ($name). The adapter starts those the agent's own way, and
  // only with the message a turn starts with (mentionedSkills()): steered, the agent would get "$name" as plain text.
  // So such a message waits for a turn of its own, as a skill from the / menu does. The list is the one the $ menu
  // loaded; before there is one, any $name counts.
  namesOwnSkill(agent, text) {
    const entry = this.skillLists.get(agent);
    const list = entry && entry.value;
    if (!list) return ownskills.hasMention(text);
    return ownskills.mentionsIn(text, (list.skills || []).map((s) => s.name)).length > 0;
  }

  // Takes out a message whose files didn't make it into the chat's folder.
  dropEntry(c, entry) {
    const at = c.queue.indexOf(entry);
    if (at < 0) return;
    c.queue.splice(at, 1);
    this.touch(c);
  }

  // After a turn that ended normally, the queue moves on once the chat is free.
  moveOn(c) {
    if (!this.turns.has(c.id) && !this.sending.has(c.id) && this.lastOutcome.get(c.id) === 'done') this.nextQueued(c.id);
  }

  // Hands a waiting message to the agent during the turn. With an agent that can't take it, it stays queued and says
  // so (fallback). Before the turn has started (its message's files are still copied) it goes after that turn.
  // copying: the promise of a message whose own files are still being copied (hold()), which settles once they are
  // in. The message takes its turn among the steers now and goes once they are in; returns {turn} when it did.
  steerEntry(c, entry, copying = null) {
    const turn = this.turns.get(c.id);
    if (!turn || !this.canSteer(c.agent)) {
      // Decided once its files are in (hold()).
      if (copying) return null;
      entry.fallback = !this.canSteer(c.agent);
      this.touch(c);
      this.emitQueue(c);
      return null;
    }
    if (!copying) {
      entry.state = 'steering';
      entry.fallback = false;
      this.touch(c);
      this.emitQueue(c);
    }
    const wait = copying && copying.catch(() => {});
    if (copying) (turn.copyingSteers = turn.copyingSteers || new Set()).add(entry);
    // One after another, in the order the user sent them.
    turn.steering = (turn.steering || Promise.resolve())
      .then(() => wait)
      .then(() => this.deliverSteer(c, turn, entry))
      .catch((err) => this.log('hub', 'steering failed:', err && err.message));
    return { turn };
  }

  async deliverSteer(c, turn, entry) {
    // The adapter gets it once it runs the turn; a turn still loading its email can end before that.
    if (!(await turn.ready) || turn.closed || entry.state !== 'steering' || !c.queue.includes(entry)) return;
    const adapter = this.adapters.get(c.agent);
    // The agent can read the files with read_chat_file as soon as it has the message.
    if (entry.files.length) {
      this.addFiles(c, entry.files);
      this.touch(c);
    }
    const files = entry.files.length ? await this.files.forTurn(c.id, entry.files, { signal: turn.controller.signal }) : [];
    if (turn.closed || entry.state !== 'steering') return;
    const attached = files.length || entry.emails.length ? { files, emails: entry.emails } : null;
    const input = context.steerText({ text: entry.body, attached, local: this.localAgent(c) });
    let taken = false;
    try {
      taken = Boolean(
        await adapter.steer(turn.handle, {
          id: entry.id,
          input,
          text: entry.body,
          files: files.filter((f) => !f.missing).map(({ id, name, type, kind, size, path: file, inline }) => ({ id, name, type, kind, size, path: file, inline: Boolean(inline) }))
        })
      );
    } catch (err) {
      this.log('hub', 'the adapter did not take a steer:', err && err.message);
    }
    if (taken || turn.closed || entry.state !== 'steering') return;
    // The agent can't take it during this turn after all: it goes after the turn, and says why.
    entry.state = 'queued';
    entry.fallback = true;
    this.touch(c);
    this.emitQueue(c);
  }

  // The agent took in a message that steered: it joins the transcript here, where the agent read it. What the agent
  // said or thought before it ends here, so its answer to the message comes below it.
  onSteered(c, turn, id) {
    if (turn.closed || this.conversations.get(c.id) !== c) return;
    const at = c.queue.findIndex((e) => e.id === id && e.state === 'steering');
    if (at < 0) return;
    const [entry] = c.queue.splice(at, 1);
    // Read: an answer from the adapter that comes after this can't put it back (deliverSteer()).
    entry.state = 'read';
    turn.reached = true;
    const now = Date.now();
    if (turn.thinking && turn.thinking.status === 'running') {
      Object.assign(turn.thinking, { status: 'done', endedAt: now });
      this.updateItem(c, turn.thinking);
    }
    turn.thinking = null;
    if (turn.assistant && turn.assistant.status === 'streaming') {
      turn.assistant.status = 'done';
      this.updateItem(c, turn.assistant);
    }
    turn.assistant = null;
    this.addItem(c, {
      type: 'user',
      text: entry.text,
      action: null,
      steered: true,
      ...(entry.files.length ? { files: entry.files.map(shownFile) } : {}),
      ...(entry.emails.length ? { emails: entry.emails } : {})
    });
    this.emitQueue(c);
  }

  // The next waiting message (or the one named) goes as a turn of its own, when the chat is free. A message whose
  // files are still being copied holds back the ones behind it; it goes once they are in (hold()).
  nextQueued(cid, entryId = null) {
    if (this.disposed) return null;
    const c = this.conversations.get(cid);
    if (!c || this.turns.has(cid) || this.sending.has(cid) || !this.cfg.enabled(c.agent)) return null;
    const at = entryId ? c.queue.findIndex((e) => e.state === 'queued' && e.id === entryId) : c.queue.findIndex((e) => e.state === 'queued' || e.state === 'copying');
    if (at < 0 || c.queue[at].state !== 'queued') return null;
    const [entry] = c.queue.splice(at, 1);
    this.emitQueue(c);
    return this.begin(c, { body: entry.body, action: entry.action, shown: entry.shown, line: entry.text, mails: entry.emails }, entry.files);
  }

  // "Send now" on a waiting message: while the agent works it steers, otherwise it goes as the next turn.
  sendQueued(id, entryId) {
    const c = this.mustGet(id);
    if (this.disposed) throw new AgentError('stopped', 'Rukoo is closing');
    const entry = c.queue.find((e) => e.id === entryId && e.state === 'queued');
    if (!entry) throw new AgentError('invalid', 'unknown queued message');
    if (!this.cfg.enabled(c.agent)) throw new AgentError('disabled', `${c.agent} is turned off`);
    if (this.turns.has(c.id) || this.sending.has(c.id)) {
      // A quick action, or a message that starts one of the agent's own skills, waits for a turn of its own.
      if (entry.action || entry.startsSkill) throw new AgentError('busy', 'a turn is running');
      this.steerEntry(c, entry);
      return { conversationId: c.id, queued: entry.id, state: entry.state };
    }
    return this.nextQueued(c.id, entry.id);
  }

  // Takes a waiting message out of the queue: Remove, or Edit, which puts it back in the input. With restage, its
  // files are staged again for chips there; otherwise they go. A file the agent already got (a steer that came back)
  // stays with the chat. A message that is steering is the agent's already.
  async unqueue(id, entryId, { restage = false } = {}) {
    const c = this.mustGet(id);
    const at = c.queue.findIndex((e) => e.id === entryId && e.state === 'queued');
    if (at < 0) throw new AgentError('invalid', 'unknown queued message');
    const [entry] = c.queue.splice(at, 1);
    this.touch(c);
    this.emitQueue(c);
    const given = new Set(c.files.map((f) => f.id));
    const own = entry.files.filter((f) => !given.has(f.id));
    let files = [];
    // Edit stages every file again; a copy the chat already has (a steer that came back) stays where it is.
    if (restage && entry.files.length) {
      const back = new Map();
      for (const [list, keep] of [
        [own, false],
        [entry.files.filter((f) => given.has(f.id)), true]
      ]) {
        const staged = list.length ? await this.files.restage(c.id, list, { keep }) : [];
        list.forEach((f, i) => back.set(f.id, staged[i]));
      }
      files = entry.files.map((f) => back.get(f.id));
    } else if (own.length) await this.files.discard(c.id, own);
    const emails = entry.emails.map(({ id: mailId, subject, from, messageId }) => ({ id: mailId, subject, from, messageId: messageId || null }));
    return { text: entry.text, action: entry.action, files, emails };
  }

  // ---------- approvals ----------

  requestApproval(cid, { title, detail = '', fields = [], choices = [], source = 'rukoo', kind = 'runtime', key = null, mail = null, calendar = null, tool = null } = {}) {
    const c = this.mustGet(cid);
    const { card, refusal } = approvalCard({ title, detail, fields, choices, tool });
    const item = this.addItem(c, {
      type: 'approval',
      ...card,
      status: refusal ? 'denied' : 'pending',
      decision: refusal ? refusal.id : null,
      // Rukoo's decision, not the user's; a recap of the chat says so (context.recap).
      ...(refusal ? { declinedBy: 'rukoo' } : {}),
      source,
      kind,
      ...(mail ? { mail } : {}),
      ...(calendar ? { calendar } : {})
    });
    const turn = kind === 'runtime' ? this.turns.get(cid) : null;
    if (turn) {
      turn.items.add(item.id);
      turn.reached = true;
    }
    if (refusal) {
      const shownTitle = context.untag(item.title);
      this.addItem(c, { type: 'notice', text: `Declined without asking: ${shownTitle}. Its input is too long to show here in full.`, tone: 'info', code: 'approval-too-long', params: { title: shownTitle }, undo: null });
      // A runtime request gets the denial as its answer; a proposal or mail action is told with the next message.
      if (kind !== 'runtime') c.notes.push(`Rukoo declined ${kind === 'mail' ? context.unsafeInline(item.title, 'mail action') : kind === 'calendar' ? context.unsafeInline(item.title, 'calendar') : `"${oneLine(item.title)}"`} without asking the user: its input was too long to show in full. Ask again with shorter input.`);
      const declined = Promise.resolve(refusal.id);
      declined.itemId = item.id;
      return declined;
    }
    let resolve;
    const promise = new Promise((r) => {
      resolve = r;
    });
    promise.itemId = item.id;
    this.approvals.set(item.id, { cid, resolve, key: key == null ? null : String(key), turnId: turn ? turn.id : null });
    // A runtime approval outside a running turn has nobody to answer it.
    if (kind === 'runtime' && !turn) this.expireApproval(cid, item.id);
    return promise;
  }

  // itemOrKey: the approval item id, or the key the adapter passed with the request (e.g. a request id).
  expireApproval(cid, itemOrKey) {
    // A phone's chat: the card is on the phone.
    if (this.phoneHost && this.phoneHost.expireApproval(cid, itemOrKey)) return true;
    const c = this.conversations.get(cid);
    for (const [itemId, w] of this.approvals) {
      if (w.cid !== cid || (itemId !== itemOrKey && (w.key === null || w.key !== String(itemOrKey)))) continue;
      this.approvals.delete(itemId);
      const item = c && c.items.find((i) => i.id === itemId);
      if (item && item.status === 'pending') {
        item.status = 'expired';
        this.updateItem(c, item);
      }
      w.resolve('cancel');
      return true;
    }
    return false;
  }

  decide(cid, itemId, choiceId) {
    const c = this.mustGet(cid);
    const item = c.items.find((i) => i.id === itemId && i.type === 'approval');
    if (!item) throw new AgentError('invalid', 'unknown approval');
    if (item.status !== 'pending') return item;
    const choice = item.choices.find((ch) => ch.id === choiceId);
    if (!choice) throw new AgentError('invalid', 'unknown choice');
    const denied = choice.kind === 'danger' || DENY_IDS.has(choice.id);
    item.status = denied ? 'denied' : 'approved';
    item.decision = choice.id;
    this.updateItem(c, item);
    const waiter = this.approvals.get(itemId);
    this.approvals.delete(itemId);
    if (waiter) waiter.resolve(choice.id);
    if (item.kind === 'proposal') this.onProposal(c, item, !denied);
    if (item.kind === 'mail') {
      this.settleMail(c, () => this.onMailDecision(c, item, !denied), item.title);
    }
    if (item.kind === 'calendar') {
      this.settleMail(c, () => this.onCalendarDecision(c, item, !denied), item.title, 'calendar');
    }
    return item;
  }

  // Mail work whose outcome goes into the notes of the agent's next message: a decision on a mail action, or an
  // action that ran without asking and outlasted its tool call. One chain per conversation, in order; runTurn()
  // waits until the chain is done. An event the user approved goes on the same chain (kind 'calendar').
  // label names the work for a note when Rukoo quits before it is done (see dispose()).
  settleMail(c, work, label = '', kind = 'mail') {
    const before = this.mailSettling.get(c.id);
    // Nothing queued: start at once, so a decision's own note is there when decide() returns.
    const started = before ? before.then(work) : new Promise((resolve) => resolve(work()));
    const running = this.mailRunning.get(c.id) || new Set();
    const entry = { label, kind };
    running.add(entry);
    this.mailRunning.set(c.id, running);
    const op = started
      .catch((err) => {
        const e = toError(err);
        this.addItem(c, { type: 'notice', text: ERROR_TEXT[e.code], detail: e.detail, tone: 'error', code: e.code, undo: null });
      })
      .finally(() => {
        running.delete(entry);
        if (!running.size && this.mailRunning.get(c.id) === running) this.mailRunning.delete(c.id);
      });
    this.mailSettling.set(c.id, op);
    op.then(() => {
      if (this.mailSettling.get(c.id) === op) this.mailSettling.delete(c.id);
    });
    return op;
  }

  // A proposal is the agent's text as it wrote it. Email text it quoted keeps its <unsafe_content> tags when Rukoo
  // repeats the proposal, so it never reaches the agent as the user's words; the user's own line shows no tags.
  onProposal(c, item, approved) {
    if (!approved) {
      c.notes.push(`The user declined: ${item.title}.`);
      this.addItem(c, { type: 'user', text: context.untag(item.title), action: 'declined' });
      return;
    }
    // The agent may not remember the proposal (another channel, a new session), so repeat what was approved.
    const lines = [`Approved: ${item.title}. Go ahead and do it now, then report back briefly.`];
    if (item.detail) lines.push(`Details you proposed: ${item.detail}`);
    for (const f of item.fields || []) lines.push(`${f.label}: ${f.value}`);
    const message = { text: lines.join('\n'), display: item.title, action: 'approved' };
    if (this.turns.has(c.id) || this.sending.has(c.id)) {
      const queue = this.followUps.get(c.id) || [];
      queue.push(message);
      this.followUps.set(c.id, queue);
    } else {
      this.sendFollowUp(c.id, message);
    }
  }

  // Mail titles and results hold sender names and subjects, which come from the email: tagged as unsafe content.
  async onMailDecision(c, item, approved) {
    if (!approved) {
      c.notes.push(`The user declined: ${context.unsafeInline(item.title, 'mail action')}.`);
      this.touch(c);
      return;
    }
    const result = await tools.executeMail(this, c, item.mail || {});
    c.notes.push(`The user approved: ${context.unsafeInline(item.title, 'mail action')}. Rukoo: ${context.unsafeInline(result.text, 'mail result')}.`);
    this.touch(c);
  }

  // The card's title holds the agent's title, which it may have taken from an email or an event: tagged.
  async onCalendarDecision(c, item, approved) {
    const title = context.unsafeInline(item.title, 'calendar');
    if (!approved) {
      c.notes.push(`The user declined: ${title}. Rukoo did not add the event.`);
      this.touch(c);
      return;
    }
    const result = await tools.executeCalendar(this, c, item.calendar || {});
    c.notes.push(
      result.ok
        ? `The user approved: ${title}. Rukoo: ${context.unsafeInline(result.text, 'calendar result')}.`
        : `The user approved: ${title}, but Rukoo could not add it: ${context.unsafeInline(result.text, 'calendar result')}. Tell the user.`
    );
    this.touch(c);
  }

  // The renderer's one write to an item: the user undid an agent's draft in the composer, or redid it.
  patchItem(cid, itemId, patch) {
    const c = this.mustGet(cid);
    const item = c.items.find((i) => i.id === itemId);
    if (!item || item.type !== 'draft') throw new AgentError('invalid', 'unknown draft');
    if (!isObj(patch) || Object.keys(patch).some((k) => k !== 'undone') || typeof patch.undone !== 'boolean') {
      throw new AgentError('invalid', 'only {undone: true|false} can change');
    }
    item.undone = patch.undone;
    return this.updateItem(c, item);
  }

  // A composer an agent wrote in closed. Its draft cards keep the draft it left behind, or none, so Show still
  // opens it after a restart. A reopened draft that is saved again gets a new id (was is the old one): cards that
  // pointed at the old one follow it.
  keepDraft(composerKey, draftId, was = null) {
    const next = draftId || null;
    for (const c of this.conversations.values()) {
      for (const item of c.items) {
        if (item.type !== 'draft') continue;
        const mine = composerKey != null && String(item.composerKey || '').split(':')[0] === composerKey;
        if (!mine && !(was && item.draftId === was)) continue;
        if ((item.draftId || null) === next) continue;
        item.draftId = next;
        this.updateItem(c, item);
      }
    }
  }

  // A stored draft was saved again under a new id, or is gone (sent or discarded: draftId null), in whichever
  // window that happened. Cards that pointed at it follow.
  draftMoved(was, draftId) {
    if (was) this.keepDraft(null, draftId, was);
  }

  async undo(cid, itemId) {
    const c = this.mustGet(cid);
    const item = c.items.find((i) => i.id === itemId && i.type === 'notice');
    if (!item || !item.undo || item.undo.kind !== 'move') throw new AgentError('invalid', 'nothing to undo');
    const ids = item.undo.ids || [];
    item.undo = null;
    this.updateItem(c, item);
    // On the mail chain like any other mail work: after what was asked for before it, and the next message
    // waits for it, so the agent hears about the undo before it looks at the mail again.
    let out = { restored: 0, failed: ids.length };
    await this.settleMail(
      c,
      async () => {
        let restored = 0;
        const failed = [];
        for (const id of ids) {
          try {
            await this.engine.undoMove(id);
            restored++;
          } catch (err) {
            failed.push(err.message);
          }
        }
        const text = failed.length
          ? `Restored ${restored} of ${ids.length} email${ids.length === 1 ? '' : 's'}. ${failed[0]}`
          : `Restored ${restored} email${restored === 1 ? '' : 's'}`;
        this.addItem(c, { type: 'notice', text, tone: failed.length && !restored ? 'error' : 'info', code: null, undo: null, mail: { action: 'undo', count: restored, failed: failed.length, folder: null } });
        c.notes.push(`The user undid ${context.unsafeInline(item.text, 'mail result')}. ${text}.`);
        this.touch(c);
        out = { restored, failed: failed.length };
      },
      `Undo: ${item.text}`
    );
    return out;
  }

  // ---------- tokens and identities ----------

  issueToken({ agent, conversationId = null } = {}) {
    const token = crypto.randomBytes(32).toString('base64url');
    this.tokens.push({ token, hash: sha256(token), identity: { agent, conversationId: conversationId || null, remote: false } });
    return token;
  }

  revokeToken(token) {
    this.tokens = this.tokens.filter((t) => t.token !== token);
  }

  // Constant-time compare of sha256 digests; every entry is checked so timing does not reveal a match position.
  identify(token) {
    if (typeof token !== 'string' || !token || token.length > 512) return null;
    const hash = sha256(token);
    let found = null;
    for (const t of this.tokens) if (crypto.timingSafeEqual(t.hash, hash)) found = t.identity;
    // Clark's token only while "Let Clark use Rukoo" is on: the Tailscale listener can also be open for OpenClaw alone.
    if (this.remoteHash && crypto.timingSafeEqual(this.remoteHash, hash) && this.cfg.data.mcp.remote) found = { agent: 'clark', conversationId: null, remote: true };
    // OpenClaw's MCP client: on this computer with the local token (the loopback listener takes only tokens that
    // aren't remote), or on another computer with the remote one, which only the Tailscale listener takes.
    if (this.openclawHash && crypto.timingSafeEqual(this.openclawHash, hash)) found = { agent: 'openclaw', conversationId: null, remote: false };
    if (this.openclawRemoteHash && crypto.timingSafeEqual(this.openclawRemoteHash, hash)) found = { agent: 'openclaw', conversationId: null, remote: true };
    // A custom agent's MCP client, the same way as OpenClaw's (config.customToken).
    for (const [agent, h] of this.customHashes || []) {
      if (h.local && crypto.timingSafeEqual(h.local, hash)) found = { agent, conversationId: null, remote: false };
      if (h.remote && crypto.timingSafeEqual(h.remote, hash)) found = { agent, conversationId: null, remote: true };
    }
    return found ? { ...found } : null;
  }

  // Whether a token issued to agent for that listener is still accepted and has the hash bind stands for (a media
  // link's binding, media.js bindOf). The same hashes identify() accepts: a chat's or a Codex process's local token,
  // Clark's while "Let Clark use Rukoo" is on, OpenClaw's local or remote one. A revoked token is gone from them.
  tokenLive(agent, remote, bind) {
    if (typeof bind !== 'string' || !bind) return false;
    const hashes = [];
    if (!remote) for (const t of this.tokens) if (t.identity.agent === agent && !t.identity.remote) hashes.push(t.hash);
    if (agent === 'clark' && remote && this.remoteHash && this.cfg.data.mcp.remote) hashes.push(this.remoteHash);
    if (agent === 'openclaw') hashes.push(remote ? this.openclawRemoteHash : this.openclawHash);
    if (this.customHashes && this.customHashes.has(agent)) hashes.push(this.customHashes.get(agent)[remote ? 'remote' : 'local']);
    return hashes.some((h) => h && this.mediaLinks.bindOf(h.toString('hex')) === bind);
  }

  // Whether agent may still use Rukoo on that listener: not turned off, and for the Tailscale listener its own
  // switch on ("Let Clark use Rukoo", or OpenClaw's remote gateway while OpenClaw is on). Claude Code and Codex run
  // on this computer only.
  agentAllowed(agent, remote) {
    const data = this.cfg.data;
    const own = this.cfg.agentData(agent);
    if (!own || own.enabled === false) return false;
    if (!remote) return true;
    if (agent === 'clark') return Boolean(data.mcp.remote);
    if (agent === 'openclaw') return Boolean(data.openclaw.remote && data.openclaw.enabled !== false);
    if (isCustomId(agent)) return Boolean(own.remote);
    return false;
  }

  // The listener a media link must point at, as its Host check accepts it: 127.0.0.1 and the local port, or the
  // Tailscale address (or the name in remoteHost) and the remote port. null while that listener isn't open.
  mediaEndpoint(remote) {
    const ports = this.mcp && !this.mcp.closed ? this.mcp.ports() : { local: null, remote: null };
    if (!remote) return ports.local ? { host: '127.0.0.1', port: ports.local } : null;
    const host = this.cfg.address();
    return ports.remote && host ? { host, port: ports.remote } : null;
  }

  // Clark's bridge looks for Rukoo on every device in the tailnet and only sends its token to one that proves it
  // has the same key. Keyed with the token's hash and its own prefix, the proof never gives the token away. It
  // covers the address this Rukoo took the connection on, so a listener elsewhere that relays the challenge here
  // gets a proof for this address, not its own.
  proof(challenge, endpoint) {
    if (!this.remoteHash || !this.cfg.data.mcp.remote || !endpoint) return '';
    return crypto.createHmac('sha256', this.remoteHash).update(`rukoo-proof:${endpoint}:${challenge}`).digest('base64url');
  }

  // What the bridge needs to pick a device: whether this Rukoo has the chat, and how long the user has been away.
  hello(identity, params = {}) {
    const id = typeof params.conversation_id === 'string' ? params.conversation_id : '';
    const c = id ? this.conversations.get(id) : null;
    let idle = null;
    try {
      idle = this.deps.idleSeconds ? Number(this.deps.idleSeconds()) : null;
    } catch (_) {
      idle = null;
    }
    return {
      device: os.hostname(),
      version: this.deps.appVersion || '0.0.0',
      idle: Number.isFinite(idle) ? idle : null,
      owns: Boolean(c && identity && c.agent === identity.agent)
    };
  }

  mapThread(threadId, cid) {
    // A phone's chat is not one of the hub's: the phone host keeps its threads.
    if (this.phoneHost && this.phoneHost.owns(cid)) return this.phoneHost.mapThread(threadId, cid);
    if (threadId && this.conversations.has(cid)) this.threads.set(String(threadId), cid);
  }

  resolveConversation(identity, args = {}, meta = {}) {
    return this.resolveCall(identity, args, meta).conversation;
  }

  // The chat a tool call is for, and whether the call is bound to it: tied to it by something the model doesn't
  // pick, so that the chat's own level counts (permissionFor()).
  resolveCall(identity, args = {}, meta = {}) {
    const none = { conversation: null, bound: false };
    if (!identity) return none;
    const own = (id) => {
      const c = typeof id === 'string' && id ? this.conversations.get(id) : null;
      return c && c.agent === identity.agent ? c : null;
    };
    // A per-chat token speaks for its own chat only. A stale or forged conversation_id must not move its
    // calls into another chat of the same agent; only tokens without a chat (Codex, Clark) are routed by it.
    if (identity.conversationId) {
      const c = own(identity.conversationId);
      return c ? { conversation: c, bound: true } : none;
    }
    // Codex's token is shared, but its thread id comes from Codex in _meta, not from the model, so it counts before
    // conversation_id, as in phonehost.route().
    const codex = meta && isObj(meta['x-codex-turn-metadata']) ? meta['x-codex-turn-metadata'].thread_id : null;
    const threadId = (meta && meta.threadId) || codex;
    const mapped = threadId ? own(this.threads.get(String(threadId))) : null;
    if (mapped) return { conversation: mapped, bound: !identity.remote };
    // The model writes conversation_id, and the one running turn is only a guess.
    const byArg = own(args && args.conversation_id);
    if (byArg) return { conversation: byArg, bound: false };
    const running = [...this.turns.keys()].map((id) => this.conversations.get(id)).filter((c) => c && c.agent === identity.agent);
    return { conversation: running.length === 1 ? running[0] : null, bound: false };
  }

  // The level a tool call goes by, once it has its chat (ensureConversation()). A chat's own level, a looser one
  // than in Settings too, counts only for a call bound to that chat (resolveCall()). Codex, Hermes and OpenClaw use
  // one token for all their chats, and an agent that an email talked round while it works in an Ask chat could name
  // a Full permission one. A call found by conversation_id, by the running turn or in no chat therefore gets the
  // strictest of that chat's level, the one in Settings, and the level of every chat the agent has a turn in now.
  permissionFor(call) {
    const c = (call && call.conversation) || null;
    if (c && call.boundTo === c.id) return this.permissionOf(c);
    const levels = [this.permissionOf(c), this.permissionOf(null)];
    for (const id of this.turns.keys()) {
      const busy = this.conversations.get(id);
      if (busy && call && busy.agent === call.agent) levels.push(this.permissionOf(busy));
    }
    return strictest(levels);
  }

  // Tools that put something on screen need a conversation. A call from outside a panel chat (Clark on
  // WhatsApp, say) goes to the agent's external conversation about the open email, so a request that shows a
  // plan and then asks for approval stays in one chat. Only a new one is revealed in the panel. Tool results
  // carry its conversation_id, so the agent can keep using it.
  ensureConversation(call) {
    if (call.conversation) return call.conversation;
    const reuse = this.externalFor(call.agent);
    if (reuse) {
      call.conversation = reuse;
      return reuse;
    }
    const open = this.openMessageId();
    const c = this.create({ agent: call.agent, message: open ? { id: open } : null, origin: 'external', title: this.agentName(call.agent) });
    call.conversation = c;
    this.emitEvent({ kind: 'reveal', conversationId: c.id });
    return c;
  }

  // The agent's recent external conversation about the email that is open now (or about no email), if any.
  externalFor(agent) {
    const open = this.openMessageId();
    const m = open ? this.liveMessage(open) : null;
    const since = Date.now() - EXTERNAL_REUSE_MS;
    // The same email can sit in two accounts; a chat about one copy is not about the other.
    const account = tools.accountOfId(open) || (m && m.accountId) || null;
    const about = (c) => {
      if (!m || !c.message) return !m && !c.message;
      const own = c.message.accountId || tools.accountOfId(c.message.id);
      if (account && own && own !== account) return false;
      if (m.messageId && c.message.messageId) return normId(m.messageId) === normId(c.message.messageId);
      return c.message.id === open;
    };
    return (
      [...this.conversations.values()]
        .filter((c) => c.origin === 'external' && c.agent === agent && c.updatedAt >= since && about(c))
        .sort((a, b) => b.updatedAt - a.updatedAt)[0] || null
    );
  }

  listTools() {
    return tools.list(this);
  }

  // The skills part of Rukoo's MCP instructions, read from disk for every initialize. It fits in what is left of
  // the 2048 characters Claude Code keeps after the fixed text and the blank line before it.
  skillInstructions() {
    return skills.instructions(this.skills.list(), skills.MCP_TEXT_MAX - MCP_INSTRUCTIONS.length - 2);
  }

  // For the panel's slash commands.
  skillList() {
    return this.skills.list().map(({ name, description, source }) => ({ name, description, source }));
  }

  // A call from Claude Code or Codex in a chat a phone runs here goes to that phone's own Rukoo, so the tools act on
  // the phone's mail and screen (phonehost.js route()).
  callTool(identity, name, args, meta) {
    // A custom agent's identity was taken when its request came in. Removed, turned off or no longer allowed on this
    // listener since then (its body can come in slowly), it gets nothing.
    if (identity && isCustomId(identity.agent) && !this.agentAllowed(identity.agent, identity.remote === true)) {
      const who = this.cfg.custom(identity.agent) ? this.agentName(identity.agent) : 'This agent';
      return { content: [{ type: 'text', text: `${who} may no longer use Rukoo's tools here. The user can set it up again in Settings > Agents.` }], isError: true };
    }
    const route = this.phoneHost && this.phoneHost.route(identity, args, meta);
    if (route) return this.phoneHost.forward(route, identity, name, args, meta);
    return tools.callTool(this, identity, name, args, meta);
  }

  // ---------- UI bridge ----------

  ui(action, args = {}, timeoutMs = 10000) {
    const hasWindow = this.deps.hasWindow ? this.deps.hasWindow() : this.listenerCount('event') > 0;
    if (!hasWindow || this.disposed) return Promise.reject(new AgentError('window', 'the main window is closed'));
    const requestId = `u_${rand(10)}`;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.uiPending.delete(requestId);
        reject(new AgentError('timeout', `the window did not answer ${action}`));
      }, timeoutMs);
      this.uiPending.set(requestId, { resolve, reject, timer });
      // The deadline lets a window that only gets the request late (its panel was still loading) skip it.
      this.emitEvent({ kind: 'ui', requestId, action, args, deadline: Date.now() + timeoutMs });
    });
  }

  uiReply(requestId, ok, result) {
    const p = this.uiPending.get(requestId);
    if (!p) return false;
    this.uiPending.delete(requestId);
    clearTimeout(p.timer);
    if (ok) p.resolve(result === undefined ? null : result);
    else if (isObj(result) && typeof result.code === 'string') p.reject(new AgentError(ERROR_TEXT[result.code] ? result.code : 'unknown', result.detail || result.message || result.code));
    else p.reject(new AgentError('unknown', typeof result === 'string' ? result : 'the window reported an error'));
    return true;
  }

  // ---------- config and status (renderer API) ----------

  config() {
    const view = this.cfg.publicView();
    for (const id of ['claude', 'codex']) {
      const a = this.adapters.get(id);
      try {
        // Optional adapter hook, used as the placeholder of the program path field.
        const exe = a && typeof a.defaultExe === 'function' ? a.defaultExe() : '';
        view[id].detectedExe = typeof exe === 'string' ? exe : '';
      } catch (_) {
        view[id].detectedExe = '';
      }
    }
    view.phone.pairingActive = Boolean(this.phoneHost && this.phoneHost.pairCode());
    return view;
  }

  async updateConfig(patch) {
    const before = JSON.stringify(this.cfg.data.mcp);
    const wanted = this.wantsRemote();
    const phoneWas = this.cfg.data.phone.enabled;
    this.cfg.update(patch);
    // Allowing a remote OpenClaw gateway, or no longer, gives or takes its remote token.
    this.refreshRemoteHash();
    // The phone may no longer use this computer: its turns stop now and a code shown for pairing stops working.
    if (phoneWas && !this.cfg.data.phone.enabled && this.phoneHost) this.phoneHost.stopAll();
    // Shortcuts and the permission level change nothing about the agents themselves: the panel only needs to draw
    // them again, and Rukoo's tools read the level when they are called.
    if (isObj(patch) && Object.keys(patch).every((k) => k === 'shortcuts' || k === 'permission')) {
      this.emitEvent({ kind: 'agents', status: this.statuses });
      return this.config();
    }
    if ((JSON.stringify(this.cfg.data.mcp) !== before || this.wantsRemote() !== wanted) && this.mcp) {
      if (this.cfg.data.mcp.port !== JSON.parse(before).port) {
        await this.mcp.close();
        await this.startMcp().catch((err) => {
          this.cfg.runtime.note = `MCP server did not start: ${err.message}`;
        });
      } else {
        await this.startRemote();
      }
    }
    this.configChanged();
    return this.config();
  }

  setSecret(agent, value) {
    this.cfg.setSecret(agent, value);
    // A new key means a new token for Clark's bridge (or OpenClaw's MCP client), and the old one stops working.
    this.refreshRemoteHash();
    this.configChanged();
    return this.config();
  }

  refreshRemoteHash() {
    const token = this.cfg.remoteToken();
    this.remoteHash = token ? sha256(token) : null;
    const openclaw = this.cfg.openclawToken();
    this.openclawHash = openclaw ? sha256(openclaw) : null;
    // Empty while the user doesn't allow a remote gateway (config.openclawToken).
    const openclawRemote = this.cfg.openclawToken(true);
    this.openclawRemoteHash = openclawRemote ? sha256(openclawRemote) : null;
    // Each custom agent's two tokens; the remote one only while the user lets it use Rukoo from another computer.
    this.customHashes = new Map();
    for (const c of this.cfg.data.custom || []) {
      const local = this.cfg.customToken(c.id);
      const remote = this.cfg.customToken(c.id, true);
      this.customHashes.set(c.id, { local: local ? sha256(local) : null, remote: remote ? sha256(remote) : null });
    }
  }

  // ---------- custom agents (renderer API) ----------

  // preset: booper, runs or openai (config.js CUSTOM_PRESETS). Returns the new agent's id with the config.
  addCustomAgent(preset) {
    const id = this.cfg.addCustom({ preset });
    this.refreshRemoteHash();
    this.syncCustomAdapters();
    this.configChanged();
    return { id, config: this.config() };
  }

  // Its chats stay, as chats that can't send. Its turns stop, and its tokens stop working at once.
  async removeCustomAgent(id) {
    if (!this.cfg.custom(id)) throw new AgentError('invalid', 'unknown agent');
    const wanted = this.wantsRemote();
    for (const turn of [...this.turns.values()]) {
      const c = this.conversations.get(turn.cid);
      if (c && c.agent === id) turn.controller.abort();
    }
    this.cfg.removeCustom(id);
    this.refreshRemoteHash();
    this.syncCustomAdapters();
    if (this.wantsRemote() !== wanted && this.mcp) await this.startRemote();
    this.configChanged();
    return this.config();
  }

  // What lets a custom agent use Rukoo's tools, on the clipboard (config.customSetup): with the agent allowed on
  // another computer, Rukoo's Tailscale address and the port its Tailscale listener has; else 127.0.0.1 and the local
  // port. Returns {remote, url}, never the token.
  copyCustomSetup(id) {
    const c = this.cfg.custom(id);
    if (!c) throw new AgentError('invalid', 'unknown agent');
    const remote = Boolean(c.remote);
    const ports = this.mcp ? this.mcp.ports() : { local: null, remote: null };
    const host = remote ? this.cfg.address() : '127.0.0.1';
    const port = remote ? ports.remote : ports.local || this.cfg.data.mcp.port;
    if (!host || !port) throw new AgentError('not-configured', 'Rukoo is not listening on a Tailscale address');
    // The first copy makes the agent's tokens; from then on the same ones.
    this.cfg.ensureSeed(id);
    this.refreshRemoteHash();
    const setup = this.cfg.customSetup(id, { remote, host, port });
    if (!setup) throw new AgentError('not-configured', `${c.name} is turned off`);
    if (this.deps.clipboard) this.deps.clipboard.writeText(setup);
    return { remote, url: `http://${host}:${port}/mcp` };
  }

  // ---------- phone (renderer API, see phonehost.js) ----------

  // Shows a code to pair a phone. Turns "Let my phone use this computer" on first, which opens the Tailscale listener.
  // allowExport: the phone may fetch the accounts and agent settings once, in the next 10 minutes.
  async phonePairStart({ allowExport = false } = {}) {
    if (!this.cfg.data.phone.enabled) await this.updateConfig({ phone: { enabled: true } });
    const port = this.mcp ? this.mcp.ports().remote : null;
    const address = this.cfg.address();
    if (!port || !address) throw new AgentError('not-configured', 'Rukoo is not listening on a Tailscale address');
    return this.phoneHost.startPairing({ allowExport: allowExport === true, address, port });
  }

  phonePairCancel() {
    this.phoneHost.cancelPairing();
    return true;
  }

  phoneUnpair(id) {
    if (!this.phoneHost.unpair(id, 'desktop')) throw new AgentError('invalid', 'unknown phone');
    return this.config();
  }

  phoneAllowExport(id) {
    if (!this.phoneHost.allowExport(id)) throw new AgentError('invalid', 'unknown phone');
    return this.config();
  }

  // The /phone/ routes on the Tailscale listener (mcp.js).
  phoneRequest(entry, req, res, pathname) {
    return this.phoneHost.handle(entry, req, res, pathname);
  }

  // GET /media/v1/<token> on either listener (mcp.js): a link get_meeting_media_link gave an agent.
  mediaRequest(entry, req, res, pathname) {
    if (this.disposed) return mcpJson(res, 404, { error: 'not found' });
    return this.mediaLinks.handle(entry, req, res, pathname);
  }

  copyHermesSetup() {
    if (this.deps.clipboard) this.deps.clipboard.writeText(this.cfg.hermesSetup({ version: this.deps.appVersion }));
    return true;
  }

  // The two steps that add Rukoo's MCP server to OpenClaw: a token line for the gateway's .env file and the command
  // (AgentConfig.openclawSetup). For a gateway on this computer: 127.0.0.1 and the port the local listener really
  // has. With a remote gateway allowed: Rukoo's Tailscale address (or the host name in
  // remoteHost) and the port the Tailscale listener really has. Returns {remote, url}, so Settings can say which one
  // went on the clipboard; never the token.
  //
  // OpenClaw gets no proof step like Clark's bridge. The bridge looks for Rukoo on every device in the tailnet and
  // must not hand its token to whichever answers, so Rukoo proves first that it has the same key. OpenClaw gets one
  // fixed URL from the user, the Tailscale address of this device, and Tailscale makes sure that address belongs to
  // this device (WireGuard keys). Its MCP client also has no way to ask for a proof.
  copyOpenClawSetup() {
    const remote = Boolean(this.cfg.data.openclaw.remote);
    const ports = this.mcp ? this.mcp.ports() : { local: null, remote: null };
    if (!this.cfg.get('openclaw').key) throw new AgentError('not-configured', 'Save the gateway token first');
    const host = remote ? this.cfg.address() : '127.0.0.1';
    const port = remote ? ports.remote : ports.local || this.cfg.data.mcp.port;
    if (!host || !port) throw new AgentError('not-configured', 'Rukoo is not listening on a Tailscale address');
    const setup = this.cfg.openclawSetup({ remote, host, port });
    if (!setup) throw new AgentError('not-configured', 'Save the gateway token first');
    if (this.deps.clipboard) this.deps.clipboard.writeText(setup);
    return { remote, url: `http://${host}:${port}/mcp` };
  }

  configChanged() {
    // A new model in Settings, program or server: the lists are loaded again when the panel asks.
    this.modelLists.clear();
    this.skillLists.clear();
    for (const a of this.adapters.values()) {
      // Optional adapter hook: restart processes that run with old settings.
      if (typeof a.configChanged === 'function') Promise.resolve().then(() => a.configChanged()).catch(() => {});
    }
    this.refreshStatus({ force: true }).catch(() => {});
  }

  async agentStatus(id, force = false) {
    if (!this.cfg.hasAgent(id)) throw new AgentError('invalid', 'unknown agent');
    if (!this.cfg.enabled(id)) return { state: 'disabled', detail: '' };
    const a = this.adapters.get(id);
    if (!a) return { state: 'unknown', detail: this.adapterErrors.get(id) || 'adapter not available' };
    try {
      // force and fresh mean the same: skip the adapter's 30 s status cache (Test connection).
      const s = await withTimeout(a.status({ force, fresh: force }), 12000);
      // steer: the agent can take a message during its turn; the panel's Send offers Steer for it.
      return { state: s && STATES.has(s.state) ? s.state : 'unknown', detail: clip((s && s.detail) || '', 300), steer: this.canSteer(id) };
    } catch (err) {
      return { state: 'unknown', detail: clip(err && (err.detail || err.message), 300) };
    }
  }

  // Probes overlap: the startup check, a saved URL or key, Test. A forced probe asks with the settings as they
  // are now, so an older probe of that agent that answers after it is dropped; it may have used the old URL
  // or key. An unforced probe can answer from the adapter's cache, so it never drops a forced one.
  // Returns whether the published status changed.
  async probeStatus(id, force) {
    if (force && this.statusRound.has(id)) this.statusRound.set(id, this.statusRound.get(id) + 1);
    const round = this.statusRound.get(id);
    const s = await this.agentStatus(id, force);
    if (this.statusRound.get(id) !== round) return false;
    const changed = JSON.stringify(s) !== JSON.stringify(this.statuses[id]);
    this.statuses = { ...this.statuses, [id]: s };
    return changed;
  }

  async refreshStatus({ force = false } = {}) {
    const changed = await Promise.all(this.cfg.agentIds().map((id) => this.probeStatus(id, force)));
    if (changed.includes(true) || force) this.emitEvent({ kind: 'agents', status: this.statuses });
    return this.statuses;
  }

  status() {
    return this.refreshStatus();
  }

  async test(agent) {
    await this.probeStatus(agent, true);
    this.emitEvent({ kind: 'agents', status: this.statuses });
    return this.statuses[agent];
  }
}

// The waiting messages the panel shows: not those whose files are still being copied (hold()).
function shownQueue(list) {
  return list.filter((e) => e.state !== 'copying');
}

// A chat's waiting messages as conversations.json has them. Each waits as 'queued' after a restart: the turn a
// steering one was for is gone. One that doesn't hold together is dropped, and so is one whose files were still
// being copied when Rukoo quit: they never made it, and the user never saw it waiting.
function cleanQueue(list) {
  if (!Array.isArray(list)) return [];
  const out = [];
  for (const e of list) {
    if (!isObj(e) || typeof e.id !== 'string' || !e.id || typeof e.text !== 'string' || typeof e.body !== 'string' || e.state === 'copying') continue;
    if (out.length >= MAX_QUEUE) break;
    out.push({
      id: e.id,
      state: 'queued',
      at: Number(e.at) || 0,
      text: e.text,
      body: e.body,
      shown: typeof e.shown === 'string' ? e.shown : e.text,
      action: typeof e.action === 'string' && e.action ? e.action : null,
      files: Array.isArray(e.files) ? e.files.filter((f) => isObj(f) && typeof f.id === 'string' && typeof f.file === 'string') : [],
      emails: Array.isArray(e.emails) ? e.emails.filter((m) => isObj(m) && typeof m.id === 'string' && m.id) : [],
      fallback: false,
      startsSkill: e.startsSkill === true
    });
  }
  return out;
}

function oneLine(v) {
  return String(v == null ? '' : v).replace(/\s+/g, ' ').trim();
}

// When the chat's last message came (MESSAGE_TYPES); a chat without one counts from when it was made. Items are kept in
// the order they came, so the last message is the newest.
function lastMessageAt(c) {
  for (let i = c.items.length - 1; i >= 0; i--) {
    const item = c.items[i];
    if (MESSAGE_TYPES.has(item.type) && Number(item.at)) return Number(item.at);
  }
  return Number(c.createdAt) || Number(c.updatedAt) || 0;
}

// A model name as the agent takes it, or null for the agent's default. strict: throw on anything else (the
// renderer's input); otherwise the value is skipped (an agent's list).
function cleanModel(v, strict = true) {
  if (v == null || v === '') return null;
  const s = typeof v === 'string' ? v.trim() : '';
  if (s && s.length <= 200 && !/[\u0000-\u001f\u007f]/.test(s)) return s;
  if (strict) throw new AgentError('invalid', 'model must be a model name of at most 200 characters');
  return null;
}

// Known effort levels only, lowest first.
function cleanEfforts(list) {
  const set = new Set(Array.isArray(list) ? list : []);
  return EFFORTS.filter((e) => set.has(e));
}

// A tool chip's detail comes from the arguments the agent passed, which may be values it copied from a tool
// result with their <unsafe_content> tags. The tags are for the agent; the user sees the value.
// Hermes clips its previews on the server, which can cut a tag in half, so a dangling piece of one at the end
// goes too. Only with signs of a cut: an ellipsis at the end, or enough of the tag's name that it can't be a
// value's own text; "price <" or "n <u" stay.
const TAG_TAIL = /<\/?(u(?:n(?:s(?:a(?:f(?:e(?:_(?:c(?:o(?:n(?:t(?:e(?:n(?:t)?)?)?)?)?)?)?)?)?)?)?)?)?)?(?:\s(?:"[^"]*(?:"|$)|[^"<>])*)?(…|\.\.\.)?$/i;
function chipDetail(v) {
  let text = context.untag(v);
  const tail = TAG_TAIL.exec(text);
  if (tail && (tail[2] || (tail[1] || '').length >= 3)) text = text.slice(0, tail.index);
  return clip(oneLine(text), 200);
}

// Untrusted text inside a note: one line, clipped, in JSON quotes so it cannot pose as Rukoo's own words.

// Adapter events that show the agent got the turn's input.
const REACHED = new Set(['text', 'commentary', 'thinking', 'tool-start']);

// The user approves what the card shows, so values pass through whole up to FIELD_MAX. A longer value keeps
// FIELD_MAX characters and says how many were cut. key tells the renderer which translated label to use.
const FIELD_MAX = 20000;
const FIELD_KEYS = new Set(['command', 'folder', 'file', 'path', 'url', 'query', 'pattern', 'content', 'before', 'after', 'diff', 'reason', 'input']);
const TOOL_KINDS = new Set(['command', 'readFile', 'changeFiles', 'web', 'mcp', 'permissions', 'other']);

// Allowing an approval allows all of its input, so nothing may be left out of the card. Past 12 fields the
// rest is folded into one "Other input" field instead of being cut.
const MAX_FIELDS = 12;
function foldFields(fields) {
  if (fields.length <= MAX_FIELDS) return fields;
  const rest = fields.slice(MAX_FIELDS - 1).map((f) => `${f.label || f.key || 'input'}: ${typeof f.value === 'string' ? f.value : JSON.stringify(f.value)}`);
  return [...fields.slice(0, MAX_FIELDS - 1), { key: 'input', label: 'Other input', value: rest.join('\n') }];
}

function approvalField(f) {
  const value = String(f.value == null ? '' : f.value);
  const out = { label: clip(f.label, 80), value: value.slice(0, FIELD_MAX) };
  if (FIELD_KEYS.has(f.key)) out.key = f.key;
  if (value.length > FIELD_MAX) out.truncated = value.length - FIELD_MAX;
  return out;
}

function approvalTool(tool) {
  if (!isObj(tool)) return null;
  const out = { name: clip(tool.name, 200), kind: TOOL_KINDS.has(tool.kind) ? tool.kind : 'other' };
  if (out.kind === 'mcp') {
    out.server = clip(tool.server, 120);
    out.tool = clip(tool.tool, 200);
  }
  return out;
}

// What an approval card shows, cleaned: at most 6 choices (Allow and Deny when the adapter gave none), the fields
// folded to 12, each whole up to FIELD_MAX. refusal is the choice to answer with at once when a field had to be cut,
// or null. The phone host (phonehost.js) builds its cards with this too, so the phone shows what a desktop would.
function approvalCard({ title, detail = '', fields = [], choices = [], tool = null } = {}) {
  const cleanChoices = (Array.isArray(choices) ? choices : [])
    .filter((ch) => isObj(ch) && ch.id)
    .slice(0, 6)
    .map((ch) => ({ id: clip(ch.id, 60), label: clip(ch.label || ch.id, 60), kind: ['primary', 'danger', 'default'].includes(ch.kind) ? ch.kind : 'default' }));
  const cleanTool = approvalTool(tool);
  const shownChoices = cleanChoices.length
    ? cleanChoices
    : [
        { id: 'allow', label: 'Allow', kind: 'primary' },
        { id: 'deny', label: 'Deny', kind: 'danger' }
      ];
  const shownFields = foldFields((Array.isArray(fields) ? fields : []).filter(isObj)).map(approvalField);
  // Allow passes on the whole input, so a card that cannot show all of it is declined without asking.
  // The adapter gets its own deny choice: cancel would end a Codex turn and leave Hermes unanswered.
  const refusal = shownFields.some((f) => f.truncated)
    ? shownChoices.find((ch) => ch.id !== 'cancel' && DENY_IDS.has(ch.id)) || shownChoices.find((ch) => ch.kind === 'danger') || { id: 'cancel' }
    : null;
  const card = {
    title: clip(title || 'Approval needed', 200),
    detail: clip(detail, 2000),
    fields: shownFields,
    ...(cleanTool ? { tool: cleanTool } : {}),
    choices: shownChoices
  };
  return { card, refusal };
}

const RUKOO_LABELS = {
  get_context: 'Read the email',
  read_message: 'Read the email',
  search_mail: 'Searched mail',
  write_draft: 'Wrote the draft',
  get_draft: 'Checked the draft',
  show_plan: 'Showed a plan',
  show_sources: 'Showed sources',
  propose_action: 'Asked for approval',
  mail_action: 'Proposed a mail action',
  read_attachment: 'Read an attachment',
  read_chat_file: 'Read an attached file',
  read_skill: 'Read a skill',
  list_events: 'Checked the calendar',
  find_free_time: 'Looked for free time',
  create_event: 'Proposed an event',
  get_booking_link: 'Got the booking link',
  read_meeting_transcript: 'Read the meeting',
  search_meetings: 'Searched meetings',
  read_meeting_notes: 'Read the meeting notes',
  get_meeting_frame: 'Looked at the screen',
  get_meeting_clip: 'Looked at the screen over a stretch',
  get_meeting_media_link: 'Got a link to meeting media'
};
const COMMANDS = /^(bash|terminal|shell|powershell|command|commandexecution|exec(_command)?|run_command|local_shell|execute_code)$/i;
const BUILTINS = [
  [/^(websearch|web_search|websearchtool|search_web)$/i, 'Searched the web'],
  [/^(webfetch|web_fetch|web_extract|fetch_url|browser.*)$/i, 'Opened a web page'],
  [/^(read|read_file|view|cat)$/i, 'Read a file'],
  [/^(write|edit|multiedit|filechange|apply_patch|write_file|patch)$/i, 'Changed files'],
  [/^(task|agent|delegate_task|subagent.*|collabagenttoolcall)$/i, 'Ran a subagent'],
  [/^(glob|grep|ls|search_files)$/i, 'Looked through files'],
  [/^(todowrite|todo)$/i, 'Updated its to-do list'],
  [/^toolsearch$/i, 'Looked up tools'],
  [/^memory$/i, 'Checked its memory']
];

// Short human label for a tool chip. Rukoo's tools get their own phrasing; others show "server · tool".
function toolLabel(name) {
  const n = String(name || '');
  const bare = tools.bareName(n);
  if (tools.isRukooTool(n) && RUKOO_LABELS[bare]) return RUKOO_LABELS[bare];
  if (COMMANDS.test(n)) return 'Ran a command';
  const mcp = /^mcp__([^_].*?)__(.+)$/.exec(n) || /^mcp_([a-z0-9-]+)_(.+)$/i.exec(n);
  if (mcp) return `${mcp[1]} · ${mcp[2]}`;
  for (const [re, label] of BUILTINS) if (re.test(n)) return label;
  return n;
}

module.exports = { AgentHub, toolLabel, approvalCard, toError, cleanModel, ERROR_TEXT, MAX_ITEMS, MAX_CONVERSATIONS, MAX_TEXT };
