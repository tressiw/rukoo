'use strict';

// A small log of how syncing and the main window's list are doing, for the day new mail doesn't show and nobody saw
// why: data/logs/rukoo.log in the profile. Past about 1 MB it becomes rukoo.1.log, replacing the one before, so the
// two stay under 2 MB together. Like Rukoo Voice's app.log, one line per entry: the time in UTC, the level, where it
// comes from and what happened.
//
// It never holds subjects, addresses, previews or tokens. Callers pass counts, durations, error codes and error text,
// and scrub() takes out what still looks like an address, a token or a path in the user's home folder.
const fs = require('fs');
const os = require('os');
const path = require('path');

const MAX_BYTES = 1024 * 1024;
const LINE_MAX = 2000;
// A window's own lines (fromWindow), at most this many a minute; more than that would push out what came before.
const WINDOW_LINES = 30;
const WINDOW_KINDS = new Set(['error', 'unhandledrejection', 'refresh', 'list']);

const HOME = (() => {
  try {
    return os.homedir();
  } catch (_) {
    return '';
  }
})();

// The path and address patterns look inside one word at a time, and an address's parts have their longest lengths,
// so a window's long text without spaces (a data URL, a one-line stack) takes one pass instead of one per character.
const PATH_WORD = /[^\s()'"]+/g;
const SRC = /[\\/]src[\\/]/;
const ADDRESS_WORD = /[^\s<>()"',;:]+/g;
const ADDRESS = /[^\s<>()"',;:@]{1,64}@[^\s<>()"',;:@]{1,253}\.[a-z]{2,24}/gi;

function scrub(text, max = LINE_MAX) {
  let s = String(text ?? '');
  // A stack names files by their place on disk: from src/ on is enough to find the line.
  s = s.replace(PATH_WORD, (word) => {
    const at = word.search(SRC);
    return at < 0 ? word : word.slice(at + 1);
  });
  if (HOME) s = s.split(HOME).join('~');
  s = s.replace(ADDRESS_WORD, (word) => (word.includes('@') ? word.replace(ADDRESS, '<address>') : word));
  s = s.replace(/[A-Za-z0-9+/_=-]{32,}/g, '<redacted>');
  s = s.replace(/\s*[\r\n]+\s*/g, ' | ');
  return s.length > max ? `${s.slice(0, max)}…` : s;
}

// A text a window sent, scrubbed and then cut to max; '' for anything else. Scrubbed first, so the cut can't leave
// part of an address or token too short to recognise. INPUT_MAX bounds the work; the word it would cut in two goes.
const INPUT_MAX = 20000;
const text = (v, max) => {
  if (typeof v !== 'string') return '';
  const s = scrub(v.length > INPUT_MAX ? v.slice(0, INPUT_MAX).replace(/\S*$/, '') : v, INPUT_MAX);
  return s.length > max ? `${s.slice(0, max)}…` : s;
};
const count = (v) => (Number.isFinite(v) && v >= 0 ? Math.round(v) : null);

class DiagLog {
  constructor(dir, { maxBytes = MAX_BYTES, now = Date.now } = {}) {
    this.dir = dir;
    this.file = path.join(dir, 'rukoo.log');
    this.old = path.join(dir, 'rukoo.1.log');
    this.maxBytes = maxBytes;
    this.now = now;
    this.size = null;
    // Per window: how many lines it wrote this minute, and how many it couldn't.
    this.windows = new Map();
  }

  write(level, target, message) {
    const line = `${new Date(this.now()).toISOString()} ${String(level).toUpperCase().padEnd(5)} ${target}: ${scrub(message)}\n`;
    try {
      if (this.size === null) {
        fs.mkdirSync(this.dir, { recursive: true });
        try {
          this.size = fs.statSync(this.file).size;
        } catch (_) {
          this.size = 0;
        }
      }
      const bytes = Buffer.byteLength(line);
      if (this.size > 0 && this.size + bytes > this.maxBytes) {
        fs.renameSync(this.file, this.old);
        this.size = 0;
      }
      fs.appendFileSync(this.file, line);
      this.size += bytes;
    } catch (_) {
      // A log that can't be written (a full disk, a locked file) leaves the app as it is.
      this.size = null;
    }
  }

  info(target, message) {
    this.write('info', target, message);
  }

  warn(target, message) {
    this.write('warn', target, message);
  }

  // What a window reports (src/renderer/diag.js): { kind, message, stack, ms, failures, reason }. Only the known kinds
  // and fields, each capped, and at most WINDOW_LINES a minute per window.
  fromWindow(name, entry) {
    if (!entry || typeof entry !== 'object' || !WINDOW_KINDS.has(entry.kind)) return false;
    const now = this.now();
    const w = this.windows.get(name) || { since: now, lines: 0, dropped: 0 };
    if (now - w.since >= 60000) {
      if (w.dropped) this.write('warn', `window ${name}`, `${w.dropped} more lines left out`);
      w.since = now;
      w.lines = 0;
      w.dropped = 0;
    }
    this.windows.set(name, w);
    if (w.lines >= WINDOW_LINES) {
      w.dropped++;
      return false;
    }
    w.lines++;
    const parts = [entry.kind];
    const reason = text(entry.reason, 40);
    if (reason) parts.push(`reason=${reason}`);
    for (const key of ['ms', 'failures']) {
      const n = count(entry[key]);
      if (n !== null) parts.push(`${key}=${n}`);
    }
    const message = text(entry.message, 500);
    if (message) parts.push(message);
    const stack = text(entry.stack, 1500);
    if (stack) parts.push(`stack: ${stack}`);
    this.write(entry.kind === 'list' ? 'info' : 'warn', `window ${name}`, parts.join(' '));
    return true;
  }
}

module.exports = { DiagLog, scrub, MAX_BYTES };
