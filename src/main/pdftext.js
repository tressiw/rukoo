'use strict';

// The text of a PDF, for agents that can't open one themselves: Codex takes no PDF as input, and Hermes on another
// machine gets the file without a reader for it. A small reader for what most PDFs use. It finds objects by
// scanning, so a broken xref table doesn't matter, and reads object streams, FlateDecode, ASCIIHex and ASCII85. It
// turns glyphs into text with a font's ToUnicode map or its encoding, and walks each page's text operators in
// order, forms included. It reads no images, so a scanned PDF has no text, and it can't decrypt. Rukoo runs it in
// a process of its own (pdfread.js), with a time limit and a memory cap. Everything here is capped as well, so a
// hostile file comes back quickly and says what is missing: work and memory grow with the file, never faster, and
// a cap that is hit ends the text early and marks it partial. It never throws.

const zlib = require('zlib');
const iconv = require('iconv-lite');

// Bytes the filters of all streams together may produce.
const INFLATE_MAX = 64 * 1024 * 1024;
const PAGES_MAX = 1000;
const OPS_MAX = 3000000;
// One string can hold millions of glyphs, and one form can be drawn a million times; operators alone don't
// measure that. CONTENT_MAX: content read, a page's streams and every form each time it is drawn. It is also the
// most one page's joined content can take, so it matches INFLATE_MAX.
const GLYPHS_MAX = 10000000;
const CONTENT_MAX = 64 * 1024 * 1024;
const FORM_DEPTH = 4;
const NEST_MAX = 64;
// Items kept of one array or dictionary; the rest is read and dropped.
const ITEMS_MAX = 100000;
// Objects a document may have, plain and packed in object streams together. Each costs memory to find and keep,
// and an object stream can claim hundreds of thousands in a small file; past this the rest goes unread.
const OBJECTS_MAX = 400000;
// Codes all ToUnicode maps and CID widths of a document get together. Ranges can repeat and overlap, so this
// counts each code set, not the codes a map ends up with.
const CMAP_MAX = 500000;
// Each glyph is tested against the code space ranges.
const RANGES_MAX = 32;
// What one code can stand for, the most the CMap specification allows.
const CMAP_TEXT_MAX = 512;

class Name {
  constructor(name) {
    this.name = name;
  }
}
class Str {
  constructor(bytes) {
    this.bytes = bytes;
  }
}
class Ref {
  constructor(num) {
    this.num = num;
  }
}
class Limit extends Error {}

const nameOf = (v) => (v instanceof Name ? v.name : '');
const isWs = (c) => c === 32 || c === 10 || c === 13 || c === 9 || c === 12 || c === 0;
const isDelim = (c) => c === 40 || c === 41 || c === 60 || c === 62 || c === 91 || c === 93 || c === 123 || c === 125 || c === 47 || c === 37;
// Written so a long run of digits has one way to match: "\d+\.?\d*" tried every split of it.
const NUMBER = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)$/;

// Tokens of the PDF syntax over a latin1 string (one character per byte). Numbers come back as numbers, names as
// Name, strings as Str, and everything else (brackets, keywords, operators) as plain strings.
class Lexer {
  // pos and end come from the file (an object stream's First and offsets): kept inside the string, or a token past
  // its end would read characters that are not there for as long as end says. flags: what a cap that drops part of
  // a value marks partial, usually the Doc.
  constructor(s, pos = 0, end = s.length, flags = { partial: false }) {
    this.s = s;
    this.end = Math.max(0, Math.min(Number(end) || 0, s.length));
    this.pos = Math.max(0, Math.min(Number(pos) || 0, this.end));
    this.flags = flags;
    // Content streams have no references; skipping the look-ahead keeps them fast.
    this.refs = true;
    // Set by Doc.scan(): a string or comment that the end cuts may run on past it (see past in scan), and grown
    // says one did. ended: a token was asked for at the end, or a string stayed cut there, so a value that holds
    // either was cut.
    this.past = null;
    this.grown = false;
    this.ended = false;
  }

  skip() {
    const s = this.s;
    let i = this.pos;
    while (i < this.end) {
      const c = s.charCodeAt(i);
      if (isWs(c)) i++;
      else if (c === 37) {
        while (i < this.end && s.charCodeAt(i) !== 10 && s.charCodeAt(i) !== 13) i++;
        if (i >= this.end && this.past) {
          const eol = this.past(this, i, (from, limit) => lineEnd(s, from, limit));
          if (eol >= 0) i = eol;
        }
      } else break;
    }
    this.pos = i;
  }

  token() {
    this.skip();
    const s = this.s;
    const i = this.pos;
    if (i >= this.end) {
      this.ended = true;
      return null;
    }
    const c = s.charCodeAt(i);
    if (c === 47) {
      let j = i + 1;
      while (j < this.end && !isWs(s.charCodeAt(j)) && !isDelim(s.charCodeAt(j))) j++;
      this.pos = j;
      return new Name(s.slice(i + 1, j).replace(/#([0-9a-fA-F]{2})/g, (_, h) => String.fromCharCode(parseInt(h, 16))));
    }
    if (c === 40) return this.literal();
    if (c === 60) {
      if (i + 1 < this.end && s.charCodeAt(i + 1) === 60) {
        this.pos = i + 2;
        return '<<';
      }
      return this.hex();
    }
    if (c === 62 && i + 1 < this.end && s.charCodeAt(i + 1) === 62) {
      this.pos = i + 2;
      return '>>';
    }
    if (isDelim(c)) {
      this.pos = i + 1;
      return s[i];
    }
    let j = i;
    while (j < this.end && !isWs(s.charCodeAt(j)) && !isDelim(s.charCodeAt(j))) j++;
    this.pos = j;
    const word = s.slice(i, j);
    return NUMBER.test(word) ? Number(word) : word;
  }

  // A string of millions of characters must not become millions of string pieces: without escapes it is a slice
  // of the source, with them it is written into one buffer.
  literal() {
    const s = this.s;
    const start = this.pos + 1;
    let i = start;
    let depth = 1;
    let escapes = false;
    for (; i < this.end; i++) {
      const c = s.charCodeAt(i);
      if (c === 92) {
        escapes = true;
        i++;
      } else if (c === 40) depth++;
      else if (c === 41 && --depth === 0) break;
    }
    if (depth > 0 && this.past) {
      const close = this.past(this, i, (from, limit) => closing(s, from, limit, depth));
      if (close >= 0) {
        i = close;
        escapes = true;
        depth = 0;
      }
    }
    if (depth > 0) this.ended = true;
    const stop = Math.min(i, this.end);
    // Never past the end: a scan goes on from here, and the end can be where the next object starts.
    this.pos = Math.min(i + 1, this.end);
    if (!escapes) return new Str(s.slice(start, stop));
    const out = Buffer.allocUnsafe(stop - start);
    let n = 0;
    for (let k = start; k < stop; k++) {
      const c = s.charCodeAt(k);
      if (c !== 92) {
        out[n++] = c;
        continue;
      }
      const e = s.charCodeAt(++k);
      if (e === 110) out[n++] = 10;
      else if (e === 114) out[n++] = 13;
      else if (e === 116) out[n++] = 9;
      else if (e === 98) out[n++] = 8;
      else if (e === 102) out[n++] = 12;
      else if (e === 13) {
        if (s.charCodeAt(k + 1) === 10) k++;
      } else if (e === 10) {
        // A line continuation.
      } else if (e >= 48 && e <= 55) {
        let v = e - 48;
        for (let d = 1; d < 3 && k + 1 < stop && s.charCodeAt(k + 1) >= 48 && s.charCodeAt(k + 1) <= 55; d++) v = v * 8 + s.charCodeAt(++k) - 48;
        out[n++] = v & 255;
      } else if (k < stop) out[n++] = e;
    }
    return new Str(out.toString('latin1', 0, n));
  }

  // The closing ">" is looked for only up to the end, so one "<" can't make every token search the whole file.
  hex() {
    const s = this.s;
    let stop = this.pos + 1;
    while (stop < this.end && s.charCodeAt(stop) !== 62) stop++;
    const digits = s.slice(this.pos + 1, stop).replace(/[^0-9a-fA-F]/g, '');
    this.pos = Math.min(stop + 1, this.end);
    return new Str(Buffer.from(digits.length % 2 ? `${digits}0` : digits, 'hex').toString('latin1'));
  }
}

// Where a literal string goes on to close: the index of its ")" from i on, with depth parentheses open, or -1 if it
// does not close before end.
function closing(s, i, end, depth) {
  for (; i < end; i++) {
    const c = s.charCodeAt(i);
    if (c === 92) i++;
    else if (c === 40) depth++;
    else if (c === 41 && --depth === 0) return i;
  }
  return -1;
}

// Where a comment goes on to end: the index of the line break from i on, or -1 if there is none before end.
function lineEnd(s, i, end) {
  for (; i < end; i++) {
    const c = s.charCodeAt(i);
    if (c === 10 || c === 13) return i;
  }
  return -1;
}

// One value: a number, a reference ("12 0 R"), an array, a dictionary (a Map), null, or the token itself. Past
// NEST_MAX an item is null, and past ITEMS_MAX it is left out; either marks lex.flags partial. A dictionary entry
// whose value is null is no entry, as the PDF specification says: /Filter null is no filter.
function value(lex, tok, depth = 0) {
  if (tok === 'null') return null;
  if (typeof tok === 'number') {
    if (lex.refs && Number.isInteger(tok) && tok >= 0) {
      const save = lex.pos;
      const ended = lex.ended;
      const gen = lex.token();
      if (Number.isInteger(gen) && lex.token() === 'R') return new Ref(tok);
      lex.pos = save;
      lex.ended = ended;
    }
    return tok;
  }
  if (tok === '[') {
    const list = [];
    for (let t = lex.token(); t !== null && t !== ']'; t = lex.token()) {
      const room = list.length < ITEMS_MAX;
      if (room) list.push(depth < NEST_MAX ? value(lex, t, depth + 1) : null);
      else if (depth < NEST_MAX) value(lex, t, depth + 1);
      if (!room || depth >= NEST_MAX) lex.flags.partial = true;
    }
    return list;
  }
  if (tok === '<<') {
    const dict = new Map();
    for (let t = lex.token(); t !== null && t !== '>>'; t = lex.token()) {
      if (!(t instanceof Name)) continue;
      const v = lex.token();
      if (v === '>>' || v === null) break;
      const item = depth < NEST_MAX ? value(lex, v, depth + 1) : null;
      const room = dict.size < ITEMS_MAX || dict.has(t.name);
      if (item === null) dict.delete(t.name);
      else if (room) dict.set(t.name, item);
      if (!room || depth >= NEST_MAX) lex.flags.partial = true;
    }
    return dict;
  }
  return tok;
}

function asciiHex(data) {
  const text = data.toString('latin1');
  const stop = text.indexOf('>');
  const digits = (stop < 0 ? text : text.slice(0, stop)).replace(/[^0-9a-fA-F]/g, '');
  return Buffer.from(digits.length % 2 ? `${digits}0` : digits, 'hex');
}

// "z" stands for four zero bytes, so the data can come out four times its size: past max bytes it stops.
function ascii85(data, max) {
  let text = data.toString('latin1').replace(/^\s*<~/, '');
  const stop = text.indexOf('~>');
  if (stop >= 0) text = text.slice(0, stop);
  const out = Buffer.allocUnsafe(Math.min(text.length * 4, Math.max(max, 0)) + 4);
  let n = 0;
  let group = 0;
  let count = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    if (c === 122 && !count) {
      if (n + 4 > max) throw new Limit();
      out.fill(0, n, n + 4);
      n += 4;
      continue;
    }
    const v = c - 33;
    if (v < 0 || v > 84) continue;
    group = group * 85 + v;
    if (++count === 5) {
      if (n + 4 > max) throw new Limit();
      out.writeUInt32BE(group >>> 0, n);
      n += 4;
      group = 0;
      count = 0;
    }
  }
  if (count > 1) {
    const size = count - 1;
    for (let k = count; k < 5; k++) group = group * 85 + 84;
    const tail = group >>> 0;
    for (let k = 0; k < size; k++) out[n++] = (tail >>> (24 - 8 * k)) & 255;
  }
  return out.subarray(0, n);
}

class Doc {
  constructor(buffer) {
    this.s = buffer.toString('latin1');
    // object number → {value, start, end} (start and end of its stream data, if it has one)
    this.objects = new Map();
    this.packed = null;
    this.trailerDicts = null;
    this.inflated = 0;
    // Objects found so far, against OBJECTS_MAX.
    this.count = 0;
    // Set when a limit left part of the document unread, so its text is incomplete.
    this.partial = false;
    this.fonts = new Map();
    this.cmaps = new Map();
    // Codes the ToUnicode maps and CID widths may still set (CMAP_MAX); parseCMap() takes the Doc for it.
    this.room = CMAP_MAX;
    // A stream whose Length is an object later in the file ends at its first endstream in the first scan, and
    // its data can hold that word. Once the lengths are read, such a stream gets its own length. If that goes past
    // an endstream, the file is scanned again with the lengths, so nothing in its data passes for an object.
    const later = this.scan();
    const known = new Map();
    let again = false;
    for (const { ref, entry, stop } of later) {
      const length = this.length(ref);
      if (!this.fits(entry.start, length)) continue;
      known.set(ref, length);
      if (stop < entry.start + length) again = true;
      else entry.end = entry.start + length;
    }
    if (again) {
      this.partial = false;
      this.scan(known);
    }
  }

  // Every "n g obj" in the file. A later definition replaces an earlier one, as in an incremental update. Stream
  // data is skipped, so bytes inside it never pass for an object. A value ends at its endobj or where the next
  // object starts, whichever comes first, and the scan goes on after it: a broken object can't take in the
  // objects after it, and one that never closes is read once, not again from every header inside it.
  // A string or comment can hold "endobj" or "12 0 obj" itself, such as a title. One that such words cut runs on to
  // its own end, and the object then ends at the first endobj or header after that (past). That reading is kept
  // only when the value closes before that end and endobj or stream follows it; else the object is read as if
  // nothing ran on, so a string that a broken writer never closed still can't take in the objects after it.
  // known: stream lengths read in a first scan, by the number of their object. Without it, returns the streams
  // whose length is an object that was not there yet.
  scan(known = null) {
    const s = this.s;
    this.objects = new Map();
    this.count = 0;
    // Where object values and stream data are, as start and end pairs in file order: a trailer is never inside one.
    this.spans = [];
    const later = known ? null : [];
    // Not inside a run of digits: retried at each of its digits, a long run took quadratic time.
    const re = /(?<!\d)(\d+)\s+\d+\s+obj\b/g;
    const ahead = new RegExp(re.source, 'g');
    const bound = new RegExp(`${re.source}|endobj`, 'g');
    let endobj = -1;
    let next = -1;
    // The bytes strings and comments may still run on past where their object seemed to end, all together: four
    // times the file. Each run on spends what it looks through, also when it finds no end, so strings that never
    // close can't each look through the rest of the file, while the few a broken writer left open still leave room.
    // A string that this leaves cut makes the text partial, and spent keeps that when the object is then read as if
    // nothing ran on: a string that ran on to its end before it doesn't undo the loss.
    let room = 4 * s.length;
    let spent = false;
    // Runs a string or comment that the end of lex cut on, from from: find(from, limit) says where it ends, and
    // lex.end moves to the first endobj or header after that. Returns where it ends, or -1 where it doesn't.
    const past = (lex, from, find) => {
      if (from >= s.length) return -1;
      if (room <= 0) {
        spent = this.partial = true;
        return -1;
      }
      const limit = Math.min(s.length, from + room);
      const at = find(from, limit);
      if (at < 0) {
        room -= limit - from;
        if (limit < s.length) spent = this.partial = true;
        return -1;
      }
      bound.lastIndex = at + 1;
      const b = bound.exec(s);
      lex.end = b ? b.index : s.length;
      room -= lex.end - from;
      lex.grown = true;
      return at;
    };
    for (let m = re.exec(s); m; m = re.exec(s)) {
      if (this.count >= OBJECTS_MAX) {
        this.partial = true;
        break;
      }
      this.count++;
      const at = m.index + m[0].length;
      if (endobj < at) {
        endobj = s.indexOf('endobj', at);
        if (endobj < 0) endobj = s.length;
      }
      if (next < at) {
        ahead.lastIndex = at;
        const n = ahead.exec(s);
        next = n ? n.index : s.length;
      }
      const partial = this.partial;
      let lex = new Lexer(s, at, Math.min(endobj, next), this);
      lex.past = past;
      let entry = { value: value(lex, lex.token()), start: -1, end: -1 };
      let valueEnd = lex.pos;
      lex.skip();
      if (lex.grown) {
        if (!lex.ended && (s.startsWith('endobj', lex.pos) || s.startsWith('stream', lex.pos))) valueEnd = lex.pos;
        else {
          this.partial = partial || spent;
          lex = new Lexer(s, at, Math.min(endobj, next), this);
          entry = { value: value(lex, lex.token()), start: -1, end: -1 };
          valueEnd = lex.pos;
          lex.skip();
        }
      }
      re.lastIndex = Math.max(re.lastIndex, Math.min(valueEnd, s.length));
      if (s.startsWith('stream', lex.pos)) {
        let start = lex.pos + 6;
        if (s[start] === '\r') start++;
        if (s[start] === '\n') start++;
        let length = entry.value instanceof Map ? entry.value.get('Length') : undefined;
        const ref = length instanceof Ref ? length.num : -1;
        if (ref >= 0) length = known && known.has(ref) ? known.get(ref) : this.length(ref);
        let end = this.fits(start, length) ? start + length : -1;
        if (end < 0) {
          const stop = s.indexOf('endstream', start);
          end = stop < 0 ? s.length : stop;
          if (s[end - 1] === '\n') end--;
          if (s[end - 1] === '\r') end--;
          if (ref >= 0 && later) later.push({ ref, entry, stop: stop < 0 ? s.length : stop });
        }
        entry.start = start;
        entry.end = end;
        re.lastIndex = Math.max(re.lastIndex, end);
        this.spans.push(at, end);
      } else if (s.startsWith('endobj', lex.pos)) this.spans.push(at, lex.pos);
      this.objects.set(Number(m[1]), entry);
    }
    return later;
  }

  // A Length given as a reference: only an integer written in that object itself, not a reference again.
  length(num) {
    const e = this.objects.get(num);
    return e && e.start < 0 && Number.isInteger(e.value) ? e.value : undefined;
  }

  // A length is taken when endstream follows the data it measures.
  fits(start, length) {
    return Number.isInteger(length) && length >= 0 && /^\s*endstream/.test(this.s.slice(start + length, start + length + 40));
  }

  entry(ref) {
    if (!(ref instanceof Ref)) return null;
    const direct = this.objects.get(ref.num);
    if (direct) return direct;
    if (!this.packed) this.unpack();
    const k = this.packed.get(ref.num);
    return k === undefined ? null : this.packedEntry(k);
  }

  // A packed object, read the first time it is used. It ends where the next object of its stream starts, and
  // objects at the same offset share one reading.
  packedEntry(k) {
    const src = this.streams[this.packedIn[k]];
    const at = this.packedAt[k];
    let e = src.read.get(at);
    if (e) return e;
    let lo = 0;
    let hi = src.starts.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (src.starts[mid] <= at) lo = mid + 1;
      else hi = mid;
    }
    const lex = new Lexer(src.s, at, lo < src.starts.length ? src.starts[lo] : src.s.length, this);
    e = { value: value(lex, lex.token()), start: -1, end: -1 };
    src.read.set(at, e);
    return e;
  }

  // A chain of references ends after 8; what lies past that is not read.
  resolve(v, depth = 0) {
    if (!(v instanceof Ref)) return v;
    const e = this.entry(v);
    if (!e) return null;
    if (depth >= 8) {
      this.partial = true;
      return null;
    }
    return this.resolve(e.value, depth + 1);
  }

  // Objects kept in object streams (PDF 1.5 and later), for numbers no plain object has. A stream can claim
  // hundreds of thousands of them and the text needs few, so each is read when it is first used (entry), and
  // until then an object costs a map entry and two numbers: its stream (packedIn) and its start (packedAt).
  // First and the offsets are the file's word; an object stream whose objects would start outside it, or that
  // lists fewer than it claims, leaves the document partial.
  unpack() {
    this.packed = new Map();
    this.streams = [];
    this.packedIn = [];
    this.packedAt = [];
    for (const e of this.objects.values()) {
      if (!(e.value instanceof Map) || nameOf(e.value.get('Type')) !== 'ObjStm') continue;
      if (this.count >= OBJECTS_MAX) {
        this.partial = true;
        break;
      }
      let s;
      try {
        s = this.stream(e);
      } catch (err) {
        if (!(err instanceof Limit)) throw err;
        this.partial = true;
        break;
      }
      if (s === null) continue;
      const n = Number(this.resolve(e.value.get('N'))) || 0;
      const first = this.resolve(e.value.get('First'));
      if (!Number.isInteger(first) || first < 0 || first > s.length) {
        this.partial = true;
        continue;
      }
      const head = new Lexer(s, 0, first, this);
      head.refs = false;
      const ats = [];
      for (let i = 0; i < n; i++) {
        const num = head.token();
        const offset = head.token();
        if (!Number.isInteger(num) || !Number.isInteger(offset) || this.count >= OBJECTS_MAX) {
          this.partial = true;
          break;
        }
        this.count++;
        const at = first + offset;
        if (num < 0 || offset < 0 || at >= s.length) {
          this.partial = true;
          continue;
        }
        ats.push(at);
        if (this.objects.has(num) || this.packed.has(num)) continue;
        this.packed.set(num, this.packedAt.length);
        this.packedIn.push(this.streams.length);
        this.packedAt.push(at);
      }
      // Where the objects start, each once and in order, for packedEntry to find where one ends.
      const starts = Float64Array.from(ats).sort();
      let unique = 0;
      for (let i = 0; i < starts.length; i++) if (!unique || starts[i] !== starts[unique - 1]) starts[unique++] = starts[i];
      this.streams.push({ s, starts: starts.subarray(0, unique), read: new Map() });
    }
  }

  // A stream's data with its filters undone, as a latin1 string; null for filters this reader does not do
  // (images, LZW, encryption), and the document is partial then. Without filters it is a slice of the file, so
  // nothing is copied.
  stream(e) {
    if (!e || e.start < 0) return null;
    const dict = e.value instanceof Map ? e.value : new Map();
    // A null in the list, or a reference to a null object, is no filter either.
    const filters = [].concat(this.resolve(dict.get('Filter')) || []).map((f) => this.resolve(f)).filter((f) => f !== null).map(nameOf);
    if (!filters.length) return this.s.slice(e.start, e.end);
    let data = Buffer.from(this.s.slice(e.start, e.end), 'latin1');
    for (const f of filters) {
      if (f === 'FlateDecode' || f === 'Fl') data = this.inflate(data);
      else if (f === 'ASCIIHexDecode' || f === 'AHx') data = this.spend(asciiHex(data));
      else if (f === 'ASCII85Decode' || f === 'A85') data = this.spend(ascii85(data, INFLATE_MAX - this.inflated));
      else {
        this.partial = true;
        return null;
      }
    }
    return data.toString('latin1');
  }

  // Every filter's output counts against INFLATE_MAX, not only Flate's.
  spend(data) {
    this.inflated += data.length;
    if (this.inflated > INFLATE_MAX) throw new Limit();
    return data;
  }

  // A stream that would inflate past what is left uses it all up: the next one stops at once instead of
  // inflating the same bomb again.
  inflate(data) {
    const room = INFLATE_MAX - this.inflated;
    if (room <= 0) throw new Limit();
    let out;
    const opts = { finishFlush: zlib.constants.Z_SYNC_FLUSH, maxOutputLength: room };
    const tooLarge = (err) => {
      if (!err || err.code !== 'ERR_BUFFER_TOO_LARGE') return;
      this.inflated = INFLATE_MAX;
      throw new Limit();
    };
    try {
      out = zlib.inflateSync(data, opts);
    } catch (err) {
      tooLarge(err);
      try {
        // Some writers leave out the zlib header.
        out = zlib.inflateRawSync(data, opts);
      } catch (rawErr) {
        tooLarge(rawErr);
        // Data that inflates neither way: whatever it held is not read.
        this.partial = true;
        out = Buffer.alloc(0);
      }
    }
    this.inflated += out.length;
    return out;
  }

  // The trailer dictionaries, first to last. "trailer" in stream data or in an object's strings is passed over: a
  // page that shows trailer<</Encrypt 9 0 R>> is no encrypted file. The search goes on after each one's value, so
  // a trailer that never closes is read once, not again for every "trailer" inside it.
  trailers() {
    if (this.trailerDicts) return this.trailerDicts;
    const s = this.s;
    const spans = this.spans;
    this.trailerDicts = [];
    let k = 0;
    for (let at = s.indexOf('trailer'); at >= 0; ) {
      while (k < spans.length && spans[k + 1] <= at) k += 2;
      if (k < spans.length && spans[k] <= at) {
        at = s.indexOf('trailer', spans[k + 1]);
        continue;
      }
      const lex = new Lexer(s, at + 7, s.length, this);
      const t = value(lex, lex.token());
      if (t instanceof Map) this.trailerDicts.push(t);
      at = lex.pos < s.length ? s.indexOf('trailer', Math.max(at + 7, lex.pos)) : -1;
    }
    return this.trailerDicts;
  }

  // Encrypted when an Encrypt leads to an encryption dictionary: a dictionary, not a stream's, whose Filter names
  // its security handler. One that leads nowhere encrypts nothing, such as a trailer that a broken object let the
  // scan find.
  encrypted() {
    const handler = (v) => {
      let dict = v;
      if (v instanceof Ref) {
        const e = this.entry(v);
        if (!e || e.start >= 0) return false;
        dict = e.value;
      }
      return dict instanceof Map && this.resolve(dict.get('Filter')) instanceof Name;
    };
    for (const t of this.trailers()) if (handler(t.get('Encrypt'))) return true;
    for (const e of this.objects.values()) if (e.value instanceof Map && nameOf(e.value.get('Type')) === 'XRef' && handler(e.value.get('Encrypt'))) return true;
    return false;
  }

  catalog() {
    const roots = [];
    for (const t of this.trailers()) if (t.get('Root')) roots.push(t.get('Root'));
    for (const e of this.objects.values()) if (e.value instanceof Map && nameOf(e.value.get('Type')) === 'XRef' && e.value.get('Root')) roots.push(e.value.get('Root'));
    for (const r of roots.reverse()) {
      const root = this.resolve(r);
      if (root instanceof Map && root.get('Pages')) return root;
    }
    for (const e of this.objects.values()) if (e.value instanceof Map && nameOf(e.value.get('Type')) === 'Catalog') return e.value;
    return null;
  }

  // The pages in order, each with the resources it has or inherits. Past PAGES_MAX, or a page tree more than 64
  // deep, the document is partial.
  pages() {
    const out = [];
    const seen = new Set();
    let deep = false;
    const walk = (node, inherited, depth) => {
      if (!(node instanceof Map)) return;
      if (out.length >= PAGES_MAX) {
        this.partial = true;
        return;
      }
      if (depth > 64) {
        deep = true;
        return;
      }
      const resources = node.has('Resources') ? this.resolve(node.get('Resources')) : inherited;
      const kids = this.resolve(node.get('Kids'));
      if (Array.isArray(kids) && nameOf(node.get('Type')) !== 'Page') {
        for (const kid of kids) {
          if (kid instanceof Ref) {
            if (seen.has(kid.num)) continue;
            seen.add(kid.num);
          }
          walk(this.resolve(kid), resources, depth + 1);
        }
        return;
      }
      out.push({ node, resources });
    };
    const root = this.catalog();
    if (root) walk(this.resolve(root.get('Pages')), null, 0);
    // A branch too deep is lost, unless the tree gave no page at all: the scan below then finds every page.
    if (deep && out.length) this.partial = true;
    if (!out.length) {
      const nums = [...this.objects.keys()].sort((a, b) => a - b);
      for (const num of nums) {
        const v = this.objects.get(num).value;
        if (!(v instanceof Map) || nameOf(v.get('Type')) !== 'Page') continue;
        if (out.length < PAGES_MAX) out.push({ node: v, resources: this.resolve(v.get('Resources')) });
        else this.partial = true;
      }
    }
    return out;
  }

  // A font written into the resources directly is kept by its dictionary, or every Tf would read it again.
  font(ref) {
    const dict = this.resolve(ref);
    const key = ref instanceof Ref ? ref.num : dict instanceof Map ? dict : null;
    if (key !== null && this.fonts.has(key)) return this.fonts.get(key);
    const font = dict instanceof Map ? new Font(this, dict) : null;
    if (key !== null) this.fonts.set(key, font);
    return font;
  }

  // A ToUnicode map, read once however many fonts share it.
  cmap(e) {
    if (this.cmaps.has(e)) return this.cmaps.get(e);
    let cmap = null;
    try {
      const data = this.stream(e);
      if (data !== null) cmap = parseCMap(data, this);
    } catch (err) {
      if (err instanceof Limit) throw err;
    }
    this.cmaps.set(e, cmap);
    return cmap;
  }
}

// ---------- fonts ----------

let tables = null;
function baseTables() {
  if (tables) return tables;
  const all = Buffer.from(Array.from({ length: 256 }, (_, i) => i));
  const win = [...iconv.decode(all, 'win1252')];
  const mac = [...iconv.decode(all, 'macintosh')];
  // StandardEncoding agrees with WinAnsi on letters and digits; its quotes differ.
  const std = win.slice();
  std[0x27] = '’';
  std[0x60] = '‘';
  for (let i = 0; i < 32; i++) win[i] = mac[i] = std[i] = '';
  tables = { WinAnsiEncoding: win, MacRomanEncoding: mac, StandardEncoding: std };
  return tables;
}

const GLYPHS = {
  space: ' ', exclam: '!', quotedbl: '"', numbersign: '#', dollar: '$', percent: '%', ampersand: '&', quotesingle: "'",
  parenleft: '(', parenright: ')', asterisk: '*', plus: '+', comma: ',', hyphen: '-', period: '.', slash: '/',
  zero: '0', one: '1', two: '2', three: '3', four: '4', five: '5', six: '6', seven: '7', eight: '8', nine: '9',
  colon: ':', semicolon: ';', less: '<', equal: '=', greater: '>', question: '?', at: '@', bracketleft: '[',
  backslash: '\\', bracketright: ']', asciicircum: '^', underscore: '_', grave: '`', braceleft: '{', bar: '|',
  braceright: '}', asciitilde: '~', quoteright: '’', quoteleft: '‘', quotedblleft: '“',
  quotedblright: '”', quotesinglbase: '‚', quotedblbase: '„', endash: '–', emdash: '—',
  bullet: '•', ellipsis: '…', fi: 'fi', fl: 'fl', ff: 'ff', ffi: 'ffi', ffl: 'ffl', trademark: '™',
  copyright: '©', registered: '®', degree: '°', Euro: '€', euro: '€', section: '§',
  paragraph: '¶', minus: '−', multiply: '×', divide: '÷', nbspace: ' ', nonbreakingspace: ' ',
  sterling: '£', yen: '¥', cent: '¢', guillemotleft: '«', guillemotright: '»', germandbls: 'ß',
  ae: 'æ', AE: 'Æ', oe: 'œ', OE: 'Œ', oslash: 'ø', Oslash: 'Ø', dotlessi: 'ı',
  lslash: 'ł', Lslash: 'Ł', eth: 'ð', Eth: 'Ð', thorn: 'þ', Thorn: 'Þ', dagger: '†',
  daggerdbl: '‡', periodcentered: '·', exclamdown: '¡', questiondown: '¿', perthousand: '‰',
  florin: 'ƒ', mu: 'µ', plusminus: '±', onehalf: '½', onequarter: '¼', threequarters: '¾'
};
const ACCENTS = { acute: '́', grave: '̀', circumflex: '̂', tilde: '̃', dieresis: '̈', ring: '̊', cedilla: '̧', caron: '̌' };

// A glyph name to text: the common names, letters with accents ("eacute"), and uniXXXX or uXXXX[XX].
function glyphText(name) {
  const n = String(name || '').split('.')[0];
  if (Object.hasOwn(GLYPHS, n)) return GLYPHS[n];
  if (/^[A-Za-z]$/.test(n)) return n;
  let m = /^uni((?:[0-9A-Fa-f]{4})+)$/.exec(n);
  if (m) return m[1].match(/.{4}/g).map((h) => String.fromCharCode(parseInt(h, 16))).join('');
  m = /^u([0-9A-Fa-f]{4,6})$/.exec(n);
  if (m) {
    const cp = parseInt(m[1], 16);
    return cp <= 0x10ffff ? String.fromCodePoint(cp) : '';
  }
  m = /^([A-Za-z])(acute|grave|circumflex|tilde|dieresis|ring|cedilla|caron)$/.exec(n);
  if (m) return (m[1] + ACCENTS[m[2]]).normalize('NFC');
  return '';
}

const utf16 = (bytes) => {
  let out = '';
  for (let i = 0; i + 1 < bytes.length; i += 2) out += String.fromCharCode((bytes.charCodeAt(i) << 8) | bytes.charCodeAt(i + 1));
  return bytes.length === 1 ? bytes : out;
};
// Codes of different lengths can share a value (<20> and <0020>), so the length is part of the key.
const codeKey = (len, code) => len * 2 ** 32 + code;
// A code in a CMap is one to four bytes. Longer ones are no code: past the safe integers, a loop over a range
// of them never moved on.
const cmapCode = (t) => (t instanceof Str && t.bytes.length >= 1 && t.bytes.length <= 4 ? read(t.bytes, 0, t.bytes.length) : -1);

// A ToUnicode CMap: code → text, and the code lengths its code space allows. Read with the lexer, in one pass:
// patterns over the text backtracked on a block that never ends. codes.room is how many codes may still be set;
// a document's maps share it (the Doc). codes.partial: a cap left codes out, so a font may have lost text: the
// room ran out, a code of more than four bytes, a range past RANGES_MAX or 65536 codes, or text past
// CMAP_TEXT_MAX.
function parseCMap(text, codes = { room: CMAP_MAX, partial: false }) {
  const map = new Map();
  const ranges = [];
  const lengths = new Set();
  const set = (len, code, mapped) => {
    map.set(codeKey(len, code), mapped);
    codes.room--;
  };
  const cut = () => (codes.partial = true);
  const textOf = (t) => {
    if (t.bytes.length > CMAP_TEXT_MAX) cut();
    return utf16(t.bytes.slice(0, CMAP_TEXT_MAX));
  };
  const lex = new Lexer(text, 0, text.length, codes);
  lex.refs = false;
  for (let tok = lex.token(); tok !== null && codes.room > 0; tok = lex.token()) {
    if (tok === 'begincodespacerange') {
      for (let a = lex.token(); a instanceof Str; a = lex.token()) {
        const lo = cmapCode(a);
        const hi = cmapCode(lex.token());
        if (lo < 0 || hi < 0 || ranges.length >= RANGES_MAX) {
          cut();
          continue;
        }
        ranges.push({ len: a.bytes.length, lo, hi });
        lengths.add(a.bytes.length);
      }
    } else if (tok === 'beginbfchar') {
      for (let a = lex.token(); a instanceof Str && codes.room > 0; a = lex.token()) {
        const code = cmapCode(a);
        const dst = lex.token();
        if (code < 0 || !(dst instanceof Str)) {
          cut();
          continue;
        }
        lengths.add(a.bytes.length);
        set(a.bytes.length, code, textOf(dst));
      }
    } else if (tok === 'beginbfrange') {
      for (let a = lex.token(); a instanceof Str && codes.room > 0; a = lex.token()) {
        const lo = cmapCode(a);
        const hi = cmapCode(lex.token());
        let dst = lex.token();
        if (dst === '[') dst = value(lex, dst);
        if (lo < 0 || hi < lo) {
          cut();
          continue;
        }
        const len = a.bytes.length;
        lengths.add(len);
        const last = Math.min(hi, lo + 65535);
        if (hi > last) cut();
        if (Array.isArray(dst)) {
          for (let i = 0; i < dst.length && lo + i <= last && codes.room > 0; i++) if (dst[i] instanceof Str) set(len, lo + i, textOf(dst[i]));
          continue;
        }
        const base = dst instanceof Str ? textOf(dst) : '';
        if (!base) {
          cut();
          continue;
        }
        const head = base.slice(0, -1);
        const tail = base.charCodeAt(base.length - 1);
        for (let c = lo; c <= last && codes.room > 0; c++) set(len, c, head + String.fromCharCode(tail + (c - lo)));
      }
    }
  }
  if (codes.room <= 0) codes.partial = true;
  return { map, ranges, lengths: [...lengths].sort((a, b) => a - b) };
}

class Font {
  constructor(doc, dict) {
    this.doc = doc;
    const subtype = nameOf(doc.resolve(dict.get('Subtype')));
    this.composite = subtype === 'Type0';
    const encoding = doc.resolve(dict.get('Encoding'));
    const tu = doc.entry(dict.get('ToUnicode'));
    this.cmap = tu ? doc.cmap(tu) : null;
    if (this.composite) {
      // Encodings such as UniJIS-UCS2-H use Unicode for codes; Identity-H needs the ToUnicode map.
      this.ucs2 = /UCS2|UTF16/i.test(nameOf(encoding));
      const desc = doc.resolve([].concat(doc.resolve(dict.get('DescendantFonts')) || [])[0]);
      this.dw = desc instanceof Map && typeof doc.resolve(desc.get('DW')) === 'number' ? doc.resolve(desc.get('DW')) : 1000;
      this.cidWidths = new Map();
      const w = desc instanceof Map ? doc.resolve(desc.get('W')) : null;
      // CIDs go up to 65535, and each width set costs one of the document's codes, as in parseCMap. Widths only
      // place the text, so one left out costs no text.
      const codes = doc;
      if (Array.isArray(w)) {
        for (let i = 0; i < w.length && codes.room > 0; ) {
          const first = doc.resolve(w[i]);
          const next = doc.resolve(w[i + 1]);
          if (!Number.isInteger(first) || first < 0 || first > 0xffff) break;
          if (Array.isArray(next)) {
            for (let k = 0; k < next.length && codes.room > 0; k++, codes.room--) this.cidWidths.set(first + k, Number(doc.resolve(next[k])) || 0);
            i += 2;
          } else {
            const width = Number(doc.resolve(w[i + 2])) || 0;
            const last = Math.min(Number(next), first + 65535);
            for (let c = first; c <= last && codes.room > 0; c++, codes.room--) this.cidWidths.set(c, width);
            i += 3;
          }
        }
      }
      this.known = true;
      return;
    }
    const t = baseTables();
    let table = subtype === 'TrueType' ? t.WinAnsiEncoding : t.StandardEncoding;
    let differences = null;
    if (encoding instanceof Name && t[encoding.name]) table = t[encoding.name];
    else if (encoding instanceof Map) {
      const base = nameOf(doc.resolve(encoding.get('BaseEncoding')));
      if (t[base]) table = t[base];
      differences = doc.resolve(encoding.get('Differences'));
    }
    this.table = table.slice();
    if (Array.isArray(differences)) {
      let code = 0;
      for (const d of differences) {
        const v = doc.resolve(d);
        if (typeof v === 'number') code = v;
        else if (v instanceof Name && code >= 0 && code < 256) this.table[code++] = glyphText(v.name);
      }
    }
    const widths = doc.resolve(dict.get('Widths'));
    this.first = Number(doc.resolve(dict.get('FirstChar'))) || 0;
    this.widths = Array.isArray(widths) ? widths.map((v) => Number(doc.resolve(v)) || 0) : null;
    const descriptor = doc.resolve(dict.get('FontDescriptor'));
    this.missing = descriptor instanceof Map ? Number(doc.resolve(descriptor.get('MissingWidth'))) || 0 : 0;
    // Type 3 glyphs are measured in their own space.
    const matrix = doc.resolve(dict.get('FontMatrix'));
    this.scale = subtype === 'Type3' && Array.isArray(matrix) ? (Number(doc.resolve(matrix[0])) || 0.001) * 1000 : 1;
    // Without widths (the standard 14 fonts in older files) a glyph counts as half an em; see Page.show().
    this.known = Boolean(this.widths);
  }

  // The length of the code at i in a string, by the code space of the ToUnicode map or the font type.
  codeLength(bytes, i) {
    const cmap = this.cmap;
    const lengths = cmap && cmap.lengths.length ? cmap.lengths : null;
    if (!lengths) return this.composite ? 2 : 1;
    if (lengths.length === 1) return lengths[0];
    for (const l of lengths) {
      const code = read(bytes, i, l);
      if (cmap.ranges.some((r) => r.len === l && code >= r.lo && code <= r.hi) || cmap.map.has(codeKey(l, code))) return l;
    }
    return lengths[lengths.length - 1];
  }

  // {text, width (in em, summed), count (glyphs), spaces (single-byte code 32, which gets word spacing)}. Glyph
  // by glyph, so a string of millions of them never becomes a list: decoding stops once the text is longer than
  // max, and each glyph counts against budget.glyphs.
  decode(bytes, max = Infinity, budget = { glyphs: 0 }) {
    let text = '';
    let width = 0;
    let spaces = 0;
    let count = 0;
    for (let i = 0; i < bytes.length && text.length <= max; count++) {
      if (++budget.glyphs > GLYPHS_MAX) throw new Limit();
      const len = this.codeLength(bytes, i);
      const code = read(bytes, i, len);
      i += len;
      const mapped = this.cmap ? this.cmap.map.get(codeKey(len, code)) : undefined;
      if (this.composite) {
        text += mapped !== undefined ? mapped : this.ucs2 ? String.fromCharCode(code) : '';
        width += (this.cidWidths.has(code) ? this.cidWidths.get(code) : this.dw) / 1000;
      } else {
        text += mapped !== undefined ? mapped : this.table[code] || '';
        const w = this.widths && code >= this.first && code - this.first < this.widths.length ? this.widths[code - this.first] : this.widths ? this.missing : 500;
        width += (w * this.scale) / 1000;
      }
      if (len === 1 && code === 32) spaces++;
    }
    return { text, width, count, spaces };
  }
}

function read(bytes, at, len) {
  let code = 0;
  for (let k = 0; k < len; k++) code = code * 256 + (bytes.charCodeAt(at + k) || 0);
  return code;
}

// ---------- pages ----------

const mul = (m, n) => [
  m[0] * n[0] + m[1] * n[2],
  m[0] * n[1] + m[1] * n[3],
  m[2] * n[0] + m[3] * n[2],
  m[2] * n[1] + m[3] * n[3],
  m[4] * n[0] + m[5] * n[2] + n[4],
  m[4] * n[1] + m[5] * n[3] + n[5]
];
const IDENTITY = [1, 0, 0, 1, 0, 0];
const matrix = (list) => (Array.isArray(list) && list.length === 6 && list.every((v) => typeof v === 'number') ? list : null);

// ---------- inline images ----------

// Every operator of content streams, to tell content from the bytes of an inline image.
const OPERATORS = new Set(['b', 'B', 'b*', 'B*', 'BDC', 'BI', 'BMC', 'BT', 'BX', 'c', 'cm', 'CS', 'cs', 'd', 'd0', 'd1', 'Do', 'DP', 'EI', 'EMC', 'ET', 'EX', 'f', 'F', 'f*', 'G', 'g', 'gs', 'h', 'i', 'ID', 'j', 'J', 'K', 'k', 'l', 'm', 'M', 'MP', 'n', 'q', 'Q', 're', 'RG', 'rg', 'ri', 's', 'S', 'SC', 'sc', 'SCN', 'scn', 'sh', 'T*', 'Tc', 'Td', 'TD', 'Tf', 'Tj', 'TJ', 'TL', 'Tm', 'Tr', 'Ts', 'Tw', 'Tz', 'v', 'w', 'W', 'W*', 'y', "'", '"']);
// Bytes looked at after a possible end of an inline image.
const IMAGE_CHECK = 1024;
// What zlib data of an inline image may inflate to in one step of finding its end, at least (flateEnd). Inline
// images are meant to be small, a few kilobytes.
const IMAGE_INFLATE = 256 * 1024;
// Colour components by colour space name, the short names of inline images included.
const COLOURS = new Map([['G', 1], ['DeviceGray', 1], ['CalGray', 1], ['I', 1], ['Indexed', 1], ['Separation', 1], ['RGB', 3], ['DeviceRGB', 3], ['CalRGB', 3], ['Lab', 3], ['CMYK', 4], ['DeviceCMYK', 4]]);

// The colour components of an inline image's colour space, or 0 when this reader can't tell. A name that is no
// device's is looked up in the page's ColorSpace resources.
function components(doc, cs, resources) {
  let space = cs;
  if (space instanceof Name && !COLOURS.has(space.name)) {
    const named = resources instanceof Map ? doc.resolve(resources.get('ColorSpace')) : null;
    space = named instanceof Map ? doc.resolve(named.get(space.name)) : null;
  }
  const family = nameOf(Array.isArray(space) ? doc.resolve(space[0]) : space);
  if (family === 'ICCBased') {
    const e = doc.entry(space[1]);
    const n = e && e.value instanceof Map ? doc.resolve(e.value.get('N')) : 0;
    return n === 1 || n === 3 || n === 4 ? n : 0;
  }
  if (family === 'DeviceN') {
    const names = doc.resolve(space[1]);
    return Array.isArray(names) ? names.length : 0;
  }
  return COLOURS.get(family) || 0;
}

// The size of an unfiltered inline image's data in bytes, from its dictionary (short or full keys), or -1 when it
// can't be told: a colour space or size this reader doesn't know.
function imageBytes(doc, image, resources) {
  const get = (short, long) => (image.has(short) ? image.get(short) : image.get(long));
  const mask = get('IM', 'ImageMask') === 'true';
  const bits = mask ? 1 : get('BPC', 'BitsPerComponent');
  const colours = mask ? 1 : components(doc, get('CS', 'ColorSpace'), resources);
  const width = get('W', 'Width');
  const height = get('H', 'Height');
  if (![1, 2, 4, 8, 16].includes(bits) || !colours || !Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1) return -1;
  // Each row starts on a byte.
  const bytes = Math.ceil((width * colours * bits) / 8) * height;
  return Number.isSafeInteger(bytes) ? bytes : -1;
}

// Whether an "EI" ends an inline image, by what follows it: lex reads from there, up to IMAGE_CHECK bytes. 1 for
// nothing, or content with an operator; -1 for what content can't be (a word that is no operator, too many
// operands, or operands the content ends with); 0 when no operator comes in the bytes looked at, so it can't be
// told. lex.pos is how far it looked.
function endsImage(lex) {
  lex.refs = false;
  const cut = lex.end < lex.s.length;
  let operators = 0;
  let operands = 0;
  for (let tok = lex.token(); tok !== null; tok = lex.token()) {
    if (typeof tok === 'string' && tok !== '[' && tok !== '<<' && tok !== 'true' && tok !== 'false' && tok !== 'null') {
      // A word the window cuts off can be part of an operator.
      if (cut && lex.pos >= lex.end) break;
      if (!OPERATORS.has(tok)) return -1;
      // The data of another inline image comes next, which is no content to check.
      if (tok === 'ID') return 1;
      operators++;
      operands = 0;
    } else {
      value(lex, tok);
      if (++operands > 64) return -1;
    }
  }
  if (cut) return operators ? 1 : 0;
  return operands ? -1 : 1;
}

// Runs the text operators of one page and writes what they show, with spaces and line breaks where the
// positions say so. room: the characters the page may write. Past them it stops with Limit, so a page of
// millions of characters is never built.
class Page {
  constructor(doc, budget, room = Infinity) {
    this.doc = doc;
    this.budget = budget;
    this.room = room;
    this.out = '';
    // Spaces and tabs at the end of the text so far, kept apart from out: a line break drops them without looking
    // through the page, which made a page of many lines quadratic.
    this.tail = '';
    this.pen = null;
    // Set once the page reached room: its text goes on past maxChars, which is not a limit of the reader.
    this.full = false;
  }

  // A stream's data for run(), counted against CONTENT_MAX before it is copied: a page can list one large stream
  // hundreds of times, and a form can be drawn a million times. A decoded stream counts as its larger size.
  content(e) {
    const raw = e && e.start >= 0 ? e.end - e.start : 0;
    this.spend(raw);
    const data = this.doc.stream(e);
    // Not a stream, or one this reader can't decode: its text is not read.
    if (data === null) this.doc.partial = true;
    else if (data.length > raw) this.spend(data.length - raw);
    return data;
  }

  spend(bytes) {
    this.budget.bytes += bytes;
    if (this.budget.bytes > CONTENT_MAX) throw new Limit();
  }

  emit(text) {
    if (!text) return;
    let end = text.length;
    while (end && (text.charCodeAt(end - 1) === 32 || text.charCodeAt(end - 1) === 9)) end--;
    if (end) {
      this.out += this.tail + text.slice(0, end);
      this.tail = text.slice(end);
    } else this.tail += text;
    if (this.out.length + this.tail.length > this.room) {
      this.out = (this.out + this.tail).slice(0, this.room);
      this.tail = '';
      this.full = true;
      throw new Limit();
    }
  }

  // An inline image, after BI: its dictionary, ID, then its data up to EI. The data is bytes and can hold "EI"
  // itself, so where it can, the data says where it ends (dataEnd), and EI must follow. Else the end is a guess and
  // the text partial: the first EI that content follows (endsImage), or one that might be. Without either, the rest
  // of the content can't be told apart from the image, and is not read.
  image(lex, resources) {
    const doc = this.doc;
    const s = lex.s;
    const dict = new Map();
    let tok = lex.token();
    for (; tok !== null && tok !== 'ID'; tok = lex.token()) {
      if (!(tok instanceof Name)) continue;
      const v = lex.token();
      if (v === null || v === 'ID') {
        tok = v;
        break;
      }
      dict.set(tok.name, value(lex, v));
    }
    if (tok === null) {
      doc.partial = true;
      return;
    }
    // One white-space byte separates ID from the data.
    const start = isWs(s.charCodeAt(lex.pos)) ? lex.pos + 1 : lex.pos;
    const end = this.dataEnd(s, start, dict, resources);
    const after = end < 0 ? -1 : this.tagEnd(s, end);
    if (after >= 0) {
      lex.pos = after;
      return;
    }
    doc.partial = true;
    const re = /[\0\t\n\f\r ]EI(?=[\0\t\n\f\r ]|$)/g;
    re.lastIndex = lex.pos;
    let unsure = -1;
    // Every byte the search passes counts as content read, also when it finds nothing, so images whose end is never
    // clear can't each search the rest of the page.
    let searched = lex.pos;
    for (let m = re.exec(s); ; m = re.exec(s)) {
      const reached = m ? m.index + m[0].length : s.length;
      this.spend(reached - searched);
      searched = reached;
      if (!m) break;
      const at = m.index + m[0].length;
      const ahead = new Lexer(s, at, Math.min(s.length, at + IMAGE_CHECK), { partial: false });
      const ends = endsImage(ahead);
      // Looking ahead counts as content read, so data full of EIs can't make the work grow unchecked.
      this.spend(ahead.pos - at);
      if (ends > 0) {
        lex.pos = at;
        return;
      }
      if (ends === 0 && unsure < 0) unsure = at;
    }
    lex.pos = unsure >= 0 ? unsure : s.length;
  }

  // Where an inline image's data ends, or -1 when the data can't tell. Without filters, by its size; else by its
  // first filter, the one its bytes are written in: zlib data ends where zlib stops, ASCIIHex at ">", ASCII85 at
  // "~>". Whatever is read to find it counts as content read.
  dataEnd(s, start, image, resources) {
    const filters = [].concat((image.has('F') ? image.get('F') : image.get('Filter')) ?? []).filter((f) => f !== null);
    if (!filters.length) {
      const size = imageBytes(this.doc, image, resources);
      return size >= 0 && size <= s.length - start ? start + size : -1;
    }
    const first = nameOf(filters[0]);
    if (first === 'Fl' || first === 'FlateDecode') return this.flateEnd(s, start);
    const mark = first === 'AHx' || first === 'ASCIIHexDecode' ? '>' : first === 'A85' || first === 'ASCII85Decode' ? '~>' : '';
    if (!mark) return -1;
    const at = s.indexOf(mark, start);
    this.spend((at < 0 ? s.length : at) - start);
    return at < 0 ? -1 : at + mark.length;
  }

  // Where zlib data that starts at start ends: inflated, zlib uses its own data and no more. Read in steps that
  // double, each counted as content read and its output as inflated, so neither grows faster than the data. A step
  // that fails doesn't say what it inflated, so it counts all it was allowed: IMAGE_INFLATE, or 64 bytes for each
  // byte read if that is more. -1 for data that is no zlib data or doesn't end.
  flateEnd(s, start) {
    const doc = this.doc;
    for (let take = Math.min(4096, s.length - start); ; take = Math.min(take * 2, s.length - start)) {
      this.spend(take);
      const room = INFLATE_MAX - doc.inflated;
      if (room <= 0) throw new Limit();
      const most = Math.min(room, Math.max(IMAGE_INFLATE, take * 64));
      let r = null;
      try {
        r = zlib.inflateSync(Buffer.from(s.slice(start, start + take), 'latin1'), { info: true, finishFlush: zlib.constants.Z_SYNC_FLUSH, maxOutputLength: most });
      } catch (err) {
        doc.inflated += most;
        if (doc.inflated >= INFLATE_MAX) throw new Limit();
        // Broken data has no end; data that inflates past this step's share may end in a larger one.
        if (!err || err.code !== 'ERR_BUFFER_TOO_LARGE') return -1;
      }
      if (r) {
        doc.inflated += r.buffer.length;
        if (r.engine.bytesWritten < take) return start + r.engine.bytesWritten;
      }
      if (take >= s.length - start) return -1;
    }
  }

  // Where the content goes on after the EI that must follow an image's data, past any white space, or -1. The white
  // space counts as content read, and a run is read once: images whose data ends in the same run (white) go to its
  // end at once, so white space that fills the rest of the page is not read again for every image.
  tagEnd(s, at) {
    let i = at;
    const run = this.white;
    if (run && run.s === s && at >= run.from && at <= run.to) i = run.to;
    else {
      while (i < s.length && isWs(s.charCodeAt(i))) i++;
      this.spend(i - at);
      this.white = { s, from: at, to: i };
    }
    this.spend(3);
    return s.startsWith('EI', i) && (i + 2 >= s.length || isWs(s.charCodeAt(i + 2))) ? i + 2 : -1;
  }

  breakLine(blank) {
    if (!this.out) return;
    this.tail = '';
    this.out += blank ? '\n\n' : '\n';
  }

  space() {
    if (this.out && !this.tail && !/\s/.test(this.out[this.out.length - 1])) this.tail = ' ';
  }

  show(st, bytes) {
    const font = st.font;
    // Text in a font that isn't there can't be read.
    if (!font) {
      if (bytes.length) this.doc.partial = true;
      return;
    }
    const { text, width, count, spaces } = font.decode(bytes, this.room - this.out.length - this.tail.length, this.budget);
    const trm = mul(st.tm, st.ctm);
    const x = trm[4];
    const y = trm[5];
    const height = Math.abs(st.size) * Math.hypot(trm[2], trm[3]) || 1;
    if (this.pen && text) {
      const dy = Math.abs(y - this.pen.y);
      if (dy > height * 0.6) this.breakLine(dy > height * 2.2);
      else {
        const gap = x - this.pen.x;
        // Without real widths the pen is only a guess: only a clear jump to the right is a space.
        if (gap > height * (this.pen.known ? 0.15 : 0.6) || gap < -height) this.space();
      }
    }
    this.emit(text);
    const advance = (width * st.size + st.tc * count + st.tw * spaces) * st.th;
    st.tm = mul([1, 0, 0, 1, advance, 0], st.tm);
    const end = mul(st.tm, st.ctm);
    if (text) this.pen = { x: end[4], y: end[5], known: font.known };
  }

  // s: the content, a latin1 string (Doc.stream).
  run(s, resources, ctm, depth = 0, seen = new Set()) {
    const doc = this.doc;
    const lex = new Lexer(s, 0, s.length, doc);
    lex.refs = false;
    const fonts = resources instanceof Map ? doc.resolve(resources.get('Font')) : null;
    const xobjects = resources instanceof Map ? doc.resolve(resources.get('XObject')) : null;
    let st = { ctm, tm: IDENTITY, lm: IDENTITY, font: null, size: 0, tc: 0, tw: 0, th: 1, tl: 0 };
    const stack = [];
    let args = [];
    for (let tok = lex.token(); tok !== null; tok = lex.token()) {
      if (typeof tok !== 'string' || tok === '[' || tok === '<<') {
        args.push(value(lex, tok));
        // No operator takes more than a few; past 64, the oldest go, and with them whatever they held.
        if (args.length > 64) {
          args.shift();
          doc.partial = true;
        }
        continue;
      }
      if (++this.budget.ops > OPS_MAX) throw new Limit();
      const a = args;
      args = [];
      switch (tok) {
        case 'q':
          // Past 64 saved states a q is not kept. Its Q then restores an older state, font included, so later text can
          // decode wrong or not at all: partial.
          if (stack.length < 64) stack.push({ ...st });
          else doc.partial = true;
          break;
        case 'Q':
          if (stack.length) st = { ...stack.pop(), tm: st.tm, lm: st.lm };
          break;
        case 'cm': {
          const m = matrix(a);
          if (m) st.ctm = mul(m, st.ctm);
          break;
        }
        case 'BT':
          st.tm = st.lm = IDENTITY;
          break;
        case 'Tf':
          st.font = fonts instanceof Map && a[0] instanceof Name ? doc.font(fonts.get(a[0].name)) : null;
          st.size = Number(a[1]) || 0;
          break;
        case 'Tc':
          st.tc = Number(a[0]) || 0;
          break;
        case 'Tw':
          st.tw = Number(a[0]) || 0;
          break;
        case 'Tz':
          st.th = (Number(a[0]) || 100) / 100;
          break;
        case 'TL':
          st.tl = Number(a[0]) || 0;
          break;
        case 'Td':
        case 'TD':
          if (tok === 'TD') st.tl = -(Number(a[1]) || 0);
          st.tm = st.lm = mul([1, 0, 0, 1, Number(a[0]) || 0, Number(a[1]) || 0], st.lm);
          break;
        case 'Tm': {
          const m = matrix(a);
          if (m) st.tm = st.lm = m;
          break;
        }
        case 'T*':
          st.tm = st.lm = mul([1, 0, 0, 1, 0, -st.tl], st.lm);
          break;
        case 'Tj':
          if (a[0] instanceof Str) this.show(st, a[0].bytes);
          break;
        case "'":
        case '"':
          st.tm = st.lm = mul([1, 0, 0, 1, 0, -st.tl], st.lm);
          if (tok === '"') {
            st.tw = Number(a[0]) || 0;
            st.tc = Number(a[1]) || 0;
          }
          if (a[a.length - 1] instanceof Str) this.show(st, a[a.length - 1].bytes);
          break;
        case 'TJ':
          for (const part of Array.isArray(a[0]) ? a[0] : []) {
            if (part instanceof Str) this.show(st, part.bytes);
            else if (typeof part === 'number') st.tm = mul([1, 0, 0, 1, (-part / 1000) * st.size * st.th, 0], st.tm);
          }
          break;
        case 'Do': {
          if (!(xobjects instanceof Map) || !(a[0] instanceof Name)) break;
          const ref = xobjects.get(a[0].name);
          const e = doc.entry(ref);
          if (!e || !(e.value instanceof Map) || nameOf(doc.resolve(e.value.get('Subtype'))) !== 'Form') break;
          // A form drawn inside itself would never end. One nested past FORM_DEPTH is not read, and says so.
          if (seen.has(ref.num)) break;
          if (depth >= FORM_DEPTH) {
            doc.partial = true;
            break;
          }
          const form = this.content(e);
          if (form === null) break;
          seen.add(ref.num);
          const own = doc.resolve(e.value.get('Resources'));
          const m = matrix(doc.resolve(e.value.get('Matrix')));
          this.run(form, own instanceof Map ? own : resources, m ? mul(m, st.ctm) : st.ctm, depth + 1, seen);
          seen.delete(ref.num);
          break;
        }
        case 'BI':
          this.image(lex, resources);
          break;
        default:
          break;
      }
    }
  }
}

// A page's content: one stream, or an array of them, which can itself be an object of its own. A reference that
// leads nowhere leaves the document partial.
function contentEntries(doc, raw) {
  const found = (refs) => {
    const list = refs.map((r) => doc.entry(r));
    if (list.some((e) => !e)) doc.partial = true;
    return list.filter(Boolean);
  };
  const list = raw instanceof Ref ? doc.entry(raw) : null;
  if (raw instanceof Ref && !list) doc.partial = true;
  if (list && Array.isArray(list.value)) return found(list.value);
  if (list) return [list];
  return Array.isArray(raw) ? found(raw) : [];
}

// {text, pages, truncated, partial, encrypted}. maxChars: where the text is cut. truncated: the text goes on past
// maxChars, so a larger maxChars gets more of it. partial: a limit of this reader (time, memory, pages) left part
// of the PDF unread, and no maxChars gets the rest. pages: the pages read.
function pdfText(buffer, { maxChars = 200000 } = {}) {
  const empty = { text: '', pages: 0, truncated: false, partial: false, encrypted: false };
  try {
    if (!Buffer.isBuffer(buffer) || buffer.indexOf('%PDF-') < 0 || buffer.indexOf('%PDF-') > 1024) return empty;
    const doc = new Doc(buffer);
    if (doc.encrypted()) return { ...empty, encrypted: true };
    const pages = doc.pages();
    const budget = { ops: 0, glyphs: 0, bytes: 0 };
    const parts = [];
    let size = 0;
    let truncated = false;
    let partial = false;
    for (const p of pages) {
      if (size > maxChars) {
        truncated = true;
        break;
      }
      // One character more than fits, so a page that fills the rest exactly is not counted as cut.
      const page = new Page(doc, budget, maxChars - size + 1);
      const chunks = [];
      try {
        for (const e of contentEntries(doc, p.node.get('Contents'))) {
          const data = page.content(e);
          if (data) chunks.push(data);
        }
      } catch (err) {
        if (!(err instanceof Limit)) throw err;
        partial = true;
      }
      try {
        // Content split over several streams is one stream; a split can fall between an operator's operands.
        page.run(chunks.join('\n'), p.resources, IDENTITY);
      } catch (err) {
        if (!(err instanceof Limit)) throw err;
        if (page.full) truncated = true;
        else partial = true;
      }
      const text = page.out.replace(/[ \t]+\n/g, '\n').trim();
      parts.push(text);
      size += text.length;
      if (truncated || partial) break;
    }
    let text = parts.filter(Boolean).join('\n\n').replace(/\u0000/g, '').replace(/\n{3,}/g, '\n\n');
    if (text.length > maxChars) {
      text = text.slice(0, maxChars);
      truncated = true;
    }
    return { text, pages: pages.length, truncated, partial: partial || doc.partial, encrypted: false };
  } catch (_) {
    // The reader broke on this PDF: whatever it holds was not read.
    return { ...empty, partial: true };
  }
}

const isPdf = (buffer) => Buffer.isBuffer(buffer) && buffer.slice(0, 1024).includes('%PDF-');

module.exports = { pdfText, isPdf, glyphText, parseCMap };
