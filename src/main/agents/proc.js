'use strict';

// Child process helpers shared by the CLI adapters: finding the program, a clean environment,
// reading JSON lines, and killing a process with everything it started.
const fs = require('fs');
const path = require('path');
const os = require('os');
const { spawn, execFile } = require('child_process');
const { StringDecoder } = require('string_decoder');

const WINDOWS = process.platform === 'win32';

// Shared by the adapters: a failure with an error code for the renderer (spec 4.6) and short English
// detail, which is all the user ever sees of it.
class AgentError extends Error {
  constructor(code, detail) {
    super(detail || code);
    this.code = code;
    this.detail = detail || '';
  }
}

function clip(text, max) {
  const s = String(text == null ? '' : text);
  return s.length > max ? s.slice(0, max - 1) + '…' : s;
}

// The hub passes a function or a console-like object; adapters only ever warn.
function logger(log) {
  if (typeof log === 'function') return log;
  if (log && typeof log.warn === 'function') return (...args) => log.warn(...args);
  return () => {};
}

function isFile(file) {
  try {
    return fs.statSync(file).isFile();
  } catch {
    return false;
  }
}

function isExecutable(file) {
  if (!isFile(file)) return false;
  if (WINDOWS) return true;
  try {
    fs.accessSync(file, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

// The bin folders of the Node versions nvm installed, newest version first. npm puts global packages
// such as Claude Code in the bin folder of the version that was active, so every version is searched.
function nvmBins(home, readdir) {
  const root = path.posix.join(home, '.nvm', 'versions', 'node');
  let names;
  try {
    names = readdir(root);
  } catch {
    return [];
  }
  const versions = [];
  for (const name of names) {
    const match = /^v(\d+)\.(\d+)\.(\d+)$/.exec(String(name));
    if (match) versions.push({ name: String(name), parts: match.slice(1).map(Number) });
  }
  versions.sort((a, b) => b.parts[0] - a.parts[0] || b.parts[1] - a.parts[1] || b.parts[2] - a.parts[2]);
  return versions.map(({ name }) => path.posix.join(root, name, 'bin'));
}

// Finder starts apps with a short PATH. Keep inherited precedence and add the common CLI locations
// for discovery AND child processes, so npm launchers using /usr/bin/env node can run too: Homebrew,
// ~/.local/bin, and the Node version managers Volta, fnm (its default version) and nvm. No shell
// startup files are evaluated. Other platforms keep their environment unchanged.
function cliPath(env = process.env, { platform = process.platform, home = os.homedir(), readdir = fs.readdirSync } = {}) {
  if (platform !== 'darwin') return env.PATH;
  const inherited = env.PATH ? env.PATH.split(':') : ['/usr/bin', '/bin', '/usr/sbin', '/sbin'];
  const fnmDefault = (dir) => path.posix.join(home, ...dir, 'fnm', 'aliases', 'default', 'bin');
  return [...new Set([
    ...inherited,
    path.posix.join(home, '.local', 'bin'),
    '/opt/homebrew/bin',
    '/usr/local/bin',
    path.posix.join(home, '.volta', 'bin'),
    fnmDefault(['.local', 'share']),
    fnmDefault(['Library', 'Application Support']),
    ...nvmBins(home, readdir)
  ])].join(':');
}

// Desktop Linux installations need not have `which`. Walk PATH directly there, preserving spaces
// and accepting executable symlinks and shebang scripts (such as npm's Codex launcher).
function onPath(name, { env = process.env, platform = process.platform, home = os.homedir(), readdir } = {}) {
  if (platform !== 'win32') {
    const files = (cliPath(env, { platform, home, readdir }) || '').split(':').map((dir) => path.resolve(dir || '.', name));
    return Promise.resolve([...new Set(files)].filter(isExecutable));
  }
  return new Promise((resolve) => {
    execFile('where', [name], { windowsHide: true, timeout: 5000 }, (err, stdout) => {
      if (err) return resolve([]);
      resolve(String(stdout).split(/\r?\n/).map((line) => line.trim()).filter(Boolean));
    });
  });
}

// The first existing file of the candidates, else the first PATH hit for `name` that is a real
// program. Windows npm shims need a shell; POSIX executable scripts work directly.
async function resolveExe(candidates, name) {
  for (const file of candidates) if (file && isExecutable(file)) return file;
  if (!name) return null;
  for (const file of await onPath(name)) {
    if (WINDOWS && !/\.exe$/i.test(file)) continue;
    if (isExecutable(file)) return file;
  }
  return null;
}

// Variables an agent CLI reads as "you are running inside another agent session". They leak in when
// Rukoo itself is started from a Claude Code or Codex terminal, and would confuse the child.
const SESSION_VARS = [
  'ELECTRON_RUN_AS_NODE',
  'CLAUDECODE',
  'CLAUDE_PID',
  'CLAUDE_EFFORT',
  'CLAUDE_AGENT_SDK_VERSION',
  'CLAUDE_PLUGIN_DATA',
  'CLAUDE_PLUGIN_ROOT'
];
const SESSION_PATTERNS = [
  /^CLAUDE_CODE_(ENTRYPOINT|SESSION_ID|CHILD_SESSION|SESSION_ATTENDED|MESSAGING_SOCKET|MESSAGING_TOKEN|EXECPATH|SSE_PORT)$/,
  /^CODEX_(THREAD_ID|SESSION_ID|TURN_ID|COMPANION_.*|MANAGED_BY_NPM|MANAGED_PACKAGE_ROOT|SANDBOX|SANDBOX_NETWORK_DISABLED|CI|INTERNAL_.*)$/
];

function cleanEnv(extra = {}, base = process.env, options = {}) {
  const env = {};
  for (const [key, value] of Object.entries(base)) {
    const upper = key.toUpperCase();
    if (SESSION_VARS.includes(upper) || SESSION_PATTERNS.some((re) => re.test(upper))) continue;
    env[key] = value;
  }
  for (const [key, value] of Object.entries(extra)) {
    if (value === undefined || value === null) delete env[key];
    else env[key] = String(value);
  }
  const resolvedPath = cliPath(env, options);
  if (resolvedPath !== undefined) env.PATH = resolvedPath;
  return env;
}

// Spawn directly without a shell (nothing to quote or inject), and no Windows console window.
function start(exe, args, { cwd, env } = {}) {
  const child = spawn(exe, args, { cwd, env, shell: false, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  // A write after the child died raises EPIPE on stdin; the exit handler reports the real problem.
  child.stdin.on('error', () => {});
  return child;
}

// Calls onLine(text) for every non-empty line; UTF-8 characters split across chunks stay whole.
function readLines(stream, onLine) {
  const decoder = new StringDecoder('utf8');
  let buffer = '';
  const flush = (text) => {
    const line = text.endsWith('\r') ? text.slice(0, -1) : text;
    if (line.trim()) onLine(line);
  };
  stream.on('data', (chunk) => {
    buffer += decoder.write(chunk);
    let at;
    while ((at = buffer.indexOf('\n')) !== -1) {
      const line = buffer.slice(0, at);
      buffer = buffer.slice(at + 1);
      flush(line);
    }
  });
  stream.on('end', () => {
    buffer += decoder.end();
    if (buffer) flush(buffer);
    buffer = '';
  });
}

// Like readLines, but parses each line as JSON. Lines that are not JSON go to onJunk (CLIs print the
// odd warning on stdout).
function readJsonLines(stream, onMessage, onJunk) {
  readLines(stream, (line) => {
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      if (onJunk) onJunk(line);
      return;
    }
    onMessage(message);
  });
}

// Keeps the last few lines of a stream (stderr) for error details.
function tail(stream, max = 20) {
  const lines = [];
  readLines(stream, (line) => {
    lines.push(line.length > 500 ? line.slice(0, 500) : line);
    if (lines.length > max) lines.shift();
  });
  return {
    lines,
    last: () => lines[lines.length - 1] || ''
  };
}

// Ends a process and everything it started. A plain kill on Windows is TerminateProcess on the parent
// only, which leaves the agent's shell commands running; taskkill /T takes the whole tree. Never blocks:
// taskkill is a process of its own. Resolves once the kill is done. A quit has to wait for that: taskkill
// is in the job Node puts its child processes in, which Windows ends together with Rukoo.
function killTree(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null || !child.pid) return Promise.resolve();
  if (WINDOWS) {
    return new Promise((resolve) => {
      const killer = spawn('taskkill', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
      killer.on('error', () => {
        try {
          child.kill();
        } catch {}
        resolve();
      });
      killer.on('exit', () => resolve());
    });
  }
  // A plain kill on Linux and macOS signals the agent alone, and its process group is no help either:
  // Claude Code starts its shell commands detached, in a group of their own. What ties a command to the
  // agent is its parent link, and that is gone once the agent dies. So list the tree first and end it from
  // the leaves up: while a parent still runs it reaps the child that exits, so nothing is reparented before
  // its signal or left as a zombie under a PID 1 that never reaps (a container). The agent goes last, and
  // whatever still runs at the deadline gets a SIGKILL.
  return descendants(child.pid).then(async (levels) => {
    const running = () => child.exitCode === null && child.signalCode === null;
    const deadline = Date.now() + 3000;
    // One depth at a time, deepest first, each waited for while the level above still runs to reap it. Half the
    // deadline goes to the descendants; then the agent, and the rest of the deadline for everything still there.
    const budget = deadline - 1500;
    let left = [];
    for (const level of [...levels].reverse()) {
      for (const pid of level) signal(pid, 'SIGTERM');
      // Once the descendants' half is spent the rest are signalled without waiting, so a deep tree or a slow ps
      // cannot push the agent past the quit's 5 s.
      if (Date.now() >= budget) {
        left.push(...level);
        continue;
      }
      let still = await living(level, 0, budget);
      while (still.length && Date.now() < budget) still = await living(still, 50, budget);
      left.push(...still);
    }
    if (running()) child.kill('SIGTERM');
    while ((running() || left.length) && Date.now() < deadline) left = await living(left, 50, deadline);
    if (running()) child.kill('SIGKILL');
    for (const pid of left) signal(pid, 'SIGKILL');
  });
}

// Which of pids still run, after waiting ms. A zombie has ended, though signal 0 still reaches it, so the
// states come from one ps listing, which may not run past until; signal 0 alone when ps fails or there is
// no time left for it.
function living(pids, ms = 0, until = Infinity) {
  return new Promise((resolve) => {
    setTimeout(() => {
      if (!pids.length) return resolve([]);
      const quick = () => resolve(pids.filter((pid) => signal(pid, 0)));
      const time = until - Date.now();
      if (time < 100) return quick();
      execFile('ps', ['-A', '-o', 'pid=', '-o', 'stat='], { timeout: Math.min(1000, time) }, (err, stdout) => {
        if (err) return quick();
        const states = new Map();
        for (const line of String(stdout).split('\n')) {
          const [pid, stat] = line.trim().split(/\s+/);
          if (pid) states.set(Number(pid), stat || '');
        }
        resolve(pids.filter((pid) => states.has(pid) && !states.get(pid).startsWith('Z')));
      });
    }, ms);
  });
}

// False when the process is gone (or not ours to signal). Signal 0 only asks whether it still exists.
function signal(pid, sig) {
  try {
    process.kill(pid, sig);
    return true;
  } catch {
    return false;
  }
}

// Every process below pid, by depth (children, grandchildren, ...), from one listing of the whole process table.
// Empty when ps fails or takes over a second (a quit waits 5 s for the whole kill): the agent itself still gets
// killed then. Separate -o options, because BSD ps reads "pid=,ppid=" as one header.
function descendants(pid) {
  return new Promise((resolve) => {
    execFile('ps', ['-A', '-o', 'pid=', '-o', 'ppid='], { timeout: 1000 }, (err, stdout) => {
      if (err) return resolve([]);
      const children = new Map();
      for (const line of String(stdout).split('\n')) {
        const [kid, parent] = line.trim().split(/\s+/).map(Number);
        if (!kid || !parent) continue;
        if (!children.has(parent)) children.set(parent, []);
        children.get(parent).push(kid);
      }
      const seen = new Set([pid]);
      const levels = [];
      for (let level = [pid]; level.length; ) {
        const next = [];
        for (const parent of level) {
          for (const kid of children.get(parent) || []) {
            if (seen.has(kid)) continue;
            seen.add(kid);
            next.push(kid);
          }
        }
        if (next.length) levels.push(next);
        level = next;
      }
      resolve(levels);
    });
  });
}

// Writes one JSON line; false when the pipe is already gone.
function writeJson(child, message) {
  if (!child || !child.stdin || child.stdin.destroyed || !child.stdin.writable) return false;
  try {
    child.stdin.write(JSON.stringify(message) + '\n');
    return true;
  } catch {
    return false;
  }
}

// ---------- approval cards ----------
// In "ask" mode the card is all that stands between the agent and what it runs, so values are never
// clipped here: the user sees every character that gets approved (the hub caps very long ones and says
// so). Each field has a `key` the renderer translates its label by; `label` is the English fallback.

// Input names every agent uses the same way → [key, label]. Anything else is key 'input'.
const INPUT_KEYS = {
  command: ['command', 'Command'],
  file_path: ['file', 'File'],
  notebook_path: ['file', 'File'],
  path: ['path', 'Path'],
  cwd: ['folder', 'Folder'],
  url: ['url', 'URL'],
  query: ['query', 'Query'],
  pattern: ['pattern', 'Pattern'],
  content: ['content', 'Content'],
  new_source: ['content', 'Content'],
  old_string: ['before', 'Old text'],
  new_string: ['after', 'New text']
};
// The hub keeps this many fields; the rest of an input folds into the last one.
const MAX_FIELDS = 12;

// output_mode → "Output mode", headLimit → "Head limit".
function humanize(name) {
  const words = String(name).replace(/_+/g, ' ').replace(/([a-z])([A-Z])/g, (_, a, b) => `${a} ${b.toLowerCase()}`).trim();
  return words ? words[0].toUpperCase() + words.slice(1) : String(name);
}

const fieldText = (value) => (typeof value === 'string' ? value : JSON.stringify(value, null, 2) ?? String(value));

function inputField(name, value, label) {
  const known = INPUT_KEYS[name];
  return { key: known ? known[0] : 'input', label: label || (known ? known[1] : humanize(name)), value: fieldText(value) };
}

// Every input of a tool call, the telling ones first. skip: inputs the card already shows elsewhere.
function inputFields(input, { lead = [], skip = [] } = {}) {
  if (input == null) return [];
  if (typeof input !== 'object' || Array.isArray(input)) return [{ key: 'input', label: 'Input', value: fieldText(input) }];
  const rank = (name) => (lead.includes(name) ? lead.indexOf(name) : lead.length);
  const names = Object.keys(input)
    .filter((name) => input[name] !== undefined && !skip.includes(name))
    .sort((a, b) => rank(a) - rank(b));
  const fields = names.map((name) => inputField(name, input[name]));
  if (fields.length <= MAX_FIELDS) return fields;
  const rest = Object.fromEntries(names.slice(MAX_FIELDS - 1).map((name) => [name, input[name]]));
  return [...fields.slice(0, MAX_FIELDS - 1), { key: 'input', label: 'Other input', value: JSON.stringify(rest, null, 2) }];
}

// The model's own summary repeats a field more often than not (Read's is the path); show it once.
const echoFree = (detail, fields) => (detail && fields.some((f) => f.value === detail) ? '' : detail || '');

// `tool.kind` lets the renderer word the title in the user's language; this is the English fallback.
const ASKS = { command: 'run a command', readFile: 'read a file', changeFiles: 'change files', web: 'open a web page' };
function approvalTitle(agent, tool, fallback) {
  if (tool.kind === 'mcp' && tool.server) return `${agent} wants to use ${tool.tool ? `${tool.server} · ${tool.tool}` : tool.server}`;
  if (tool.kind === 'permissions') return `${agent} wants more permissions`;
  if (ASKS[tool.kind]) return `${agent} wants to ${ASKS[tool.kind]}`;
  return `${agent} wants to use ${fallback || tool.name || 'a tool'}`;
}

module.exports = {
  WINDOWS,
  AgentError,
  clip,
  logger,
  isFile,
  onPath,
  cliPath,
  resolveExe,
  cleanEnv,
  start,
  readLines,
  readJsonLines,
  tail,
  killTree,
  writeJson,
  humanize,
  fieldText,
  inputField,
  inputFields,
  echoFree,
  approvalTitle
};
