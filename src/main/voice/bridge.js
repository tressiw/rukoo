'use strict';

// The WebSocket client of the Rukoo Voice bridge (voice/PROTOCOL.md "Finding Voice" and "Connecting"). Pure Node: the
// global WebSocket that Node and Electron's main process have, so it runs under node --test against a fake Voice.
//
// It never stops trying while a discovery file is there. Ownkey #13 is the lesson: its bridge treated one reset
// connection as fatal, stopped listening, and nothing ever started it again because the process still ran. Here
// every close schedules another attempt (1 s, doubling up to 30 s), and a changed discovery file connects at once.
//
// The token goes to Voice in hello and nowhere else: not to the log, the renderer, the store or an agent.

const fs = require('fs');
const path = require('path');
const { EventEmitter } = require('events');
const { PROTOCOL, MIN_PROTOCOL, VOICE_MESSAGE_MAX, RUKOO_MESSAGE_MAX, DISCOVERY_MAX, validate, validateDiscovery, negotiate } = require('./protocol');

const BACKOFF_MS = [1000, 2000, 4000, 8000, 16000, 30000];
// The longest wait while the discovery file names a Voice that runs. A meeting Voice starts itself waits up to 5 s
// for a Rukoo it saw before to come back, so Rukoo can hide its windows before the first frame (voice/PROTOCOL.md
// "meeting.starting").
const LIVE_RETRY_MS = 2000;
const COMMAND_TIMEOUT_MS = 10000;
const WELCOME_TIMEOUT_MS = 10000;
// Voice pings every 20 s, but a browser-style WebSocket can't see pings, so Rukoo asks for the status now and then:
// a Voice that hangs without closing the socket shows up as a command that times out.
const HEARTBEAT_MS = 30000;
const POLL_MS = 5000;

class VoiceError extends Error {
  constructor(code, message, meeting = null) {
    super(message || code);
    this.code = code;
    if (meeting) this.meeting = meeting;
  }
}

// Whether every string in v is valid UTF-16. JSON carries a lone surrogate as an escape Voice's parser refuses, and
// without the command's id Voice can't even answer bad_request.
function wellFormed(v) {
  if (typeof v === 'string') return v.isWellFormed();
  if (v && typeof v === 'object') return Object.entries(v).every(([k, x]) => k.isWellFormed() && wellFormed(x));
  return true;
}

// Whether a process with this id runs. EPERM: it runs as another user. pid 0 says nothing, so Rukoo connects to find out.
function pidAlive(pid) {
  if (!pid) return true;
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === 'EPERM';
  }
}

class VoiceBridge extends EventEmitter {
  // file: the discovery file, or null for a profile that never looks for Voice. cursor(): {instance, after} for hello.
  constructor({
    file,
    clientVersion = '',
    cursor = () => ({ instance: null, after: 0 }),
    WebSocket = globalThis.WebSocket,
    backoffMs = BACKOFF_MS,
    liveRetryMs = LIVE_RETRY_MS,
    commandTimeoutMs = COMMAND_TIMEOUT_MS,
    welcomeTimeoutMs = WELCOME_TIMEOUT_MS,
    heartbeatMs = HEARTBEAT_MS,
    pollMs = POLL_MS,
    alive = pidAlive,
    platform = process.platform,
    log = () => {}
  } = {}) {
    super();
    this.file = file || null;
    this.clientVersion = String(clientVersion || '');
    this.cursor = cursor;
    this.WebSocket = WebSocket;
    this.backoffMs = backoffMs;
    this.liveRetryMs = liveRetryMs;
    this.commandTimeoutMs = commandTimeoutMs;
    this.welcomeTimeoutMs = welcomeTimeoutMs;
    this.heartbeatMs = heartbeatMs;
    this.pollMs = pollMs;
    this.alive = alive;
    this.platform = platform;
    this.log = log;
    this.ws = null;
    this.welcomed = false;
    // The discovery file the current socket or the next attempt belongs to, as port, token, instance and pid.
    this.key = null;
    // A file Voice refused for its version: tried again only once the file changes.
    this.parked = null;
    this.attempt = 0;
    this.retryTimer = null;
    this.retryWait = null;
    this.pollTimer = null;
    this.heartbeatTimer = null;
    this.welcomeTimer = null;
    this.watcher = null;
    this.pending = new Map();
    this.nextId = 0;
    this.lastError = null;
    this.closed = false;
    this.current = { status: 'absent', reason: this.file ? 'missing' : 'none', voiceVersion: null, protocol: null, older: null };
  }

  state() {
    return { ...this.current };
  }

  get connected() {
    return Boolean(this.ws && this.welcomed);
  }

  setState(next) {
    const s = { status: next.status, reason: next.reason || null, voiceVersion: next.voiceVersion || null, protocol: next.protocol || null, older: next.older || null };
    if (JSON.stringify(s) === JSON.stringify(this.current)) return;
    this.current = s;
    this.emit('state', this.state());
  }

  start() {
    if (!this.file) {
      this.setState({ status: 'absent', reason: 'none' });
      return this;
    }
    this.watch();
    this.pollTimer = setInterval(() => {
      // The folder may only now exist; fs.watch also misses changes on some setups, so the poll checks the file too.
      if (!this.watcher) this.watch();
      this.check();
    }, this.pollMs);
    if (this.pollTimer.unref) this.pollTimer.unref();
    this.check();
    return this;
  }

  watch() {
    try {
      const name = path.basename(this.file);
      // The long form of the folder: libuv on Windows asserts, and takes the whole process down, when it watches a
      // folder named with 8.3 short names (C:\Users\BARTAD~1\...) and reports a change under the long name.
      this.watcher = fs.watch(fs.realpathSync.native(path.dirname(this.file)), (type, changed) => {
        if (!changed || String(changed) === name) this.check();
      });
      this.watcher.on('error', () => {
        if (this.watcher) this.watcher.close();
        this.watcher = null;
      });
    } catch (_) {
      // No folder yet: the poll tries again.
      this.watcher = null;
    }
  }

  // The discovery file as Rukoo may use it: {status: 'ok', discovery, protocol, key}, or the state to show.
  readDiscovery() {
    let st;
    try {
      st = fs.statSync(this.file);
    } catch (_) {
      return { status: 'absent', reason: 'missing' };
    }
    if (!st.isFile() || st.size > DISCOVERY_MAX) return { status: 'absent', reason: 'invalid' };
    // On macOS and Linux only the user may read it. Windows relies on the access list Voice gives the file.
    if (this.platform !== 'win32' && typeof process.getuid === 'function' && ((st.mode & 0o077) !== 0 || st.uid !== process.getuid())) {
      return { status: 'insecure', reason: 'permissions' };
    }
    let parsed;
    try {
      parsed = JSON.parse(fs.readFileSync(this.file, 'utf8'));
    } catch (_) {
      return { status: 'absent', reason: 'invalid' };
    }
    const checked = validateDiscovery(parsed);
    if (!checked.ok) return { status: 'absent', reason: 'invalid' };
    const d = checked.discovery;
    // A file Voice left behind when it crashed. Voice owns it, so Rukoo leaves it alone and waits for a new one.
    if (!this.alive(d.pid)) return { status: 'absent', reason: 'stale' };
    const protocol = negotiate([MIN_PROTOCOL, PROTOCOL], [d.minProtocol, d.protocol]);
    if (protocol === null) {
      return { status: 'incompatible', voiceVersion: d.voiceVersion, older: d.protocol < MIN_PROTOCOL ? 'voice' : 'rukoo' };
    }
    return { status: 'ok', discovery: d, protocol, key: [d.port, d.token, d.instance, d.pid].join('\n') };
  }

  check() {
    if (this.closed || !this.file) return;
    const d = this.readDiscovery();
    if (d.status !== 'ok') {
      // A connection that works keeps working without the file; the next attempt after it closes finds out.
      if (!this.ws && !this.retryTimer) {
        this.key = null;
        this.setState(d);
      }
      return;
    }
    if (d.key === this.parked) return;
    if (d.key === this.key && (this.ws || this.retryTimer)) return;
    // Another Voice, or Voice started again: connect now, with the backoff from the start.
    if (d.key !== this.key) this.attempt = 0;
    this.connect(d);
  }

  connect(d) {
    this.drop();
    clearTimeout(this.retryTimer);
    this.retryTimer = null;
    this.key = d.key;
    this.parked = null;
    this.lastError = null;
    this.setState({ status: 'connecting', voiceVersion: d.discovery.voiceVersion, protocol: d.protocol });
    let ws;
    try {
      // The literal address: Voice listens on 127.0.0.1 only, and localhost may resolve elsewhere.
      ws = new this.WebSocket(`ws://127.0.0.1:${d.discovery.port}/bridge`);
    } catch (err) {
      this.log('bridge', `could not open the socket: ${err.message}`);
      this.retry();
      return;
    }
    this.ws = ws;
    this.welcomed = false;
    const mine = () => this.ws === ws;
    ws.addEventListener('open', () => {
      if (!mine()) return;
      const { instance, after } = this.cursor();
      this.sendRaw({ type: 'hello', protocol: d.protocol, token: d.discovery.token, client: 'rukoo', clientVersion: this.clientVersion, pid: process.pid, instance: instance || null, after: Number.isInteger(after) && after >= 0 ? after : 0 });
      this.welcomeTimer = setTimeout(() => {
        if (!mine() || this.welcomed) return;
        this.log('bridge', 'no welcome from Voice');
        this.closeSocket();
      }, this.welcomeTimeoutMs);
    });
    ws.addEventListener('message', (e) => {
      if (mine()) this.onMessage(e.data, d);
    });
    ws.addEventListener('error', () => {
      // The close that follows says what to do.
    });
    ws.addEventListener('close', (e) => {
      if (mine()) this.onClose(e.code, d);
    });
  }

  onMessage(data, d) {
    if (typeof data !== 'string') return;
    if (data.length > VOICE_MESSAGE_MAX) {
      this.log('bridge', 'a message from Voice was larger than the protocol allows');
      this.closeSocket();
      return;
    }
    let msg;
    try {
      msg = JSON.parse(data);
    } catch (_) {
      this.log('bridge', 'a message from Voice was not JSON');
      return;
    }
    const checked = validate('voice', msg);
    if (!checked.ok) {
      // A bad event is skipped and never stops the events after it.
      this.log('bridge', `skipped a ${typeof (msg && msg.type) === 'string' ? msg.type.slice(0, 20) : 'message'} from Voice: invalid ${checked.reason}`);
      this.emit('invalid', { type: msg && msg.type, reason: checked.reason });
      if (!this.welcomed && msg && msg.type === 'welcome') this.closeSocket();
      return;
    }
    msg = checked.message;
    if (msg.type === 'error') {
      this.lastError = msg.code;
      this.log('bridge', `Voice refused the connection: ${msg.code}`);
      return;
    }
    if (!this.welcomed) {
      if (msg.type !== 'welcome') return;
      clearTimeout(this.welcomeTimer);
      this.welcomeTimer = null;
      this.welcomed = true;
      this.attempt = 0;
      this.setState({ status: 'connected', voiceVersion: msg.voiceVersion, protocol: msg.protocol });
      this.startHeartbeat();
      this.emit('welcome', msg);
      return;
    }
    if (msg.type === 'event') this.emit('event', msg);
    else if (msg.type === 'reply') this.onReply(msg);
  }

  onReply(msg) {
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    clearTimeout(p.timer);
    if (msg.ok) p.resolve(msg.result);
    else p.reject(new VoiceError(msg.error.code, msg.error.message, typeof msg.error.meeting === 'string' ? msg.error.meeting : null));
  }

  onClose(code, d) {
    const wasWelcomed = this.welcomed;
    this.ws = null;
    this.welcomed = false;
    this.stopTimers();
    this.failPending('offline');
    if (wasWelcomed) this.emit('closed');
    if (this.closed) return;
    if (code === 4002 || this.lastError === 'unsupported_protocol') {
      this.parked = this.key;
      this.setState({ status: 'incompatible', voiceVersion: d.discovery.voiceVersion, older: null });
      return;
    }
    this.retry();
  }

  retry() {
    if (this.closed) return;
    let wait = this.backoffMs[Math.min(this.attempt, this.backoffMs.length - 1)];
    // Voice runs, so it is starting its side or got stuck for a moment, and it may be waiting for Rukoo to hide its
    // windows. A Voice that turned Rukoo down (a wrong token, too many clients) keeps the backoff: asking sooner won't
    // change its answer.
    if (wait > this.liveRetryMs && !this.lastError && this.voiceRuns()) wait = this.liveRetryMs;
    // For the tests: how long the next attempt waits.
    this.retryWait = wait;
    this.attempt += 1;
    if (this.current.status !== 'connecting') this.setState({ ...this.current, status: 'connecting' });
    clearTimeout(this.retryTimer);
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      const d = this.readDiscovery();
      if (d.status !== 'ok') {
        this.key = null;
        this.setState(d);
        return;
      }
      if (d.key !== this.key) this.attempt = 0;
      this.connect(d);
    }, wait);
    if (this.retryTimer.unref) this.retryTimer.unref();
  }

  // Whether the discovery file names a process that runs. pid 0 names none.
  voiceRuns() {
    const d = this.readDiscovery();
    return d.status === 'ok' && d.discovery.pid > 0 && this.alive(d.discovery.pid);
  }

  startHeartbeat() {
    clearInterval(this.heartbeatTimer);
    if (!this.heartbeatMs) return;
    this.heartbeatTimer = setInterval(() => {
      this.command('status.get', {})
        .then((status) => this.emit('status', status))
        .catch((err) => {
          if (err.code === 'timeout') {
            this.log('bridge', 'Voice stopped answering; connecting again');
            this.closeSocket();
          }
        });
    }, this.heartbeatMs);
    if (this.heartbeatTimer.unref) this.heartbeatTimer.unref();
  }

  stopTimers() {
    clearInterval(this.heartbeatTimer);
    clearTimeout(this.welcomeTimer);
    this.heartbeatTimer = null;
    this.welcomeTimer = null;
  }

  failPending(code) {
    for (const [id, p] of this.pending) {
      clearTimeout(p.timer);
      p.reject(new VoiceError(code, code === 'offline' ? 'Rukoo Voice is not connected.' : code));
      this.pending.delete(id);
    }
  }

  sendRaw(message) {
    try {
      this.ws.send(JSON.stringify(message));
      return true;
    } catch (err) {
      this.log('bridge', `sending failed: ${err.message}`);
      return false;
    }
  }

  // A command and its reply's result. Rejects with a VoiceError: Voice's code, or offline, timeout or bad_request.
  command(name, args = {}, { timeoutMs = this.commandTimeoutMs } = {}) {
    if (!this.connected) return Promise.reject(new VoiceError('offline', 'Rukoo Voice is not connected.'));
    const id = `c-${++this.nextId}`;
    const message = { type: 'command', id, command: name, args };
    const checked = validate('rukoo', message);
    if (!checked.ok) return Promise.reject(new VoiceError('bad_request', `invalid ${checked.reason}`));
    if (!wellFormed(args)) return Promise.reject(new VoiceError('bad_request', 'text with a lone surrogate'));
    if (JSON.stringify(message).length > RUKOO_MESSAGE_MAX) return Promise.reject(new VoiceError('bad_request', 'too large'));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new VoiceError('timeout', `Rukoo Voice did not answer ${name} within ${timeoutMs} ms.`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      if (!this.sendRaw(message)) {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(new VoiceError('offline', 'Rukoo Voice is not connected.'));
      }
    });
  }

  closeSocket() {
    const ws = this.ws;
    if (!ws) return;
    try {
      ws.close(1000);
    } catch (_) {
      // Already closing.
    }
  }

  // Leaves the current socket without a reconnect from its close.
  drop() {
    const ws = this.ws;
    if (!ws) return;
    this.ws = null;
    this.welcomed = false;
    this.stopTimers();
    this.failPending('offline');
    try {
      ws.close(1000);
    } catch (_) {
      // Already closing.
    }
  }

  // Synchronous, for quit: no timers, no watcher, no socket. Voice keeps recording.
  close() {
    this.closed = true;
    clearTimeout(this.retryTimer);
    clearInterval(this.pollTimer);
    this.retryTimer = null;
    this.pollTimer = null;
    if (this.watcher) this.watcher.close();
    this.watcher = null;
    this.drop();
  }
}

module.exports = { VoiceBridge, VoiceError, pidAlive, BACKOFF_MS, LIVE_RETRY_MS };
