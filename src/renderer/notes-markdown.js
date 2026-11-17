// The notes editor supports a small Markdown vocabulary. DOM output is escaped before the
// shared sanitizer sees it; pasted HTML goes through the same serializer before it is saved.
import { sanitizeAgentHtml } from './agent/richtext.js';

// The store limits Markdown to 100,000 characters. Escaping and editor wrappers can
// expand valid notes beyond the mail HTML budget; sanitize them without clipping.
const sanitizeNotes = (html) => sanitizeAgentHtml(html, { maxInput: Infinity });

const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const safeHref = (s) => /^(https?:|mailto:)/i.test(s);
// The characters a backslash escapes.
const ESCAPABLE = '\\`*_{}[]()#+.!<>~-';
const ESCAPED = /\\([\\`*_{}\[\]()#+.!<>~-])/g;
const escapes = (text, at) => text[at] === '\\' && at + 1 < text.length && ESCAPABLE.includes(text[at + 1]);

// One line of notes as HTML. slots: the HTML of each placeholder, shared with the labels of its links.
function inline(text, slots = []) {
  return format(read(text, slots), slots);
}

// Escapes, code spans and links, read from left to right as a Markdown reader does: whichever starts first wins.
// A backtick or a backslash in a link address is part of the address, and link syntax in a code span is code.
// Returns the line with a placeholder for each of them. Every search ahead remembers how far it got, so a line takes
// time in proportion to its length, also with a long run of brackets or backticks that never close.
function read(text, slots) {
  const keep = (html) => `\u0000${slots.push(html) - 1}\u0000`;
  const parts = [];
  // i: where the reader is. The text from plain to i goes into parts as it is.
  let i = 0;
  let plain = 0;
  const flush = () => {
    if (i > plain) parts.push(text.slice(plain, i));
    plain = i;
  };
  // part takes the place of the text from i up to the index to.
  const put = (part, to) => {
    flush();
    parts.push(part);
    i = plain = to;
  };
  // Where the text of a link starts: the first [ since the last ], as an index in parts.
  let open = -1;

  // The runs of backticks by length, each with how far the search for a closing run has got.
  let runs = null;
  const closing = (length, after) => {
    if (!runs) {
      runs = new Map();
      for (const m of text.matchAll(/`+/g)) {
        if (!runs.has(m[0].length)) runs.set(m[0].length, { at: [], next: 0 });
        runs.get(m[0].length).at.push(m.index);
      }
    }
    const run = runs.get(length);
    if (!run) return -1;
    while (run.next < run.at.length && run.at[run.next] <= after) run.next++;
    return run.next < run.at.length ? run.at[run.next] : -1;
  };
  // The first > from at on, or the end of the line.
  let gt = -1;
  const nextGt = (at) => {
    if (gt < at) {
      gt = text.indexOf('>', at);
      if (gt < 0) gt = text.length;
    }
    return gt;
  };
  // Where an address without angle brackets ends: at the first space or ) from at on, after backslash escapes.
  let ends = null;
  const bareEnd = (at) => {
    if (!ends) ends = new Int32Array(text.length + 1).fill(-1);
    const passed = [];
    let p = at;
    while (p < text.length && ends[p] < 0 && text[p] !== ')' && !/\s/.test(text[p])) {
      passed.push(p);
      p += escapes(text, p) ? 2 : 1;
    }
    const end = p >= text.length ? text.length : ends[p] >= 0 ? ends[p] : p;
    for (const q of passed) ends[q] = end;
    return end;
  };
  // The address after a link's ], as [address, where the link ends], or null. The editor writes an address in angle
  // brackets as it is (notesToMarkdown), so that one has no escapes.
  const address = (at) => {
    if (text[at] !== '(') return null;
    if (text[at + 1] === '<') {
      const end = nextGt(at + 2);
      if (end > at + 2 && text[end + 1] === ')') return [text.slice(at + 2, end), end + 2];
    }
    const end = bareEnd(at + 1);
    if (end > at + 1 && text[end] === ')') return [text.slice(at + 1, end).replace(ESCAPED, '$1'), end + 1];
    return null;
  };

  while (i < text.length) {
    if (escapes(text, i)) put(keep(esc(text[i + 1])), i + 2);
    else if (text[i] === '`') {
      let end = i;
      while (text[end] === '`') end++;
      const close = closing(end - i, i);
      // Backticks without a closing run of as many are text.
      if (close < 0) i = end;
      else {
        let code = text.slice(end, close);
        if (code.startsWith(' ') && code.endsWith(' ') && code.trim()) code = code.slice(1, -1);
        put(keep(`<code>${esc(code)}</code>`), close + end - i);
      }
    } else if (text[i] === '[' && open < 0) {
      flush();
      open = parts.length;
      put('[', i + 1);
    } else if (text[i] === ']' && open >= 0) {
      flush();
      // A link needs text: [] is none. An unsafe address leaves the text without it.
      const link = parts.length > open + 1 && address(i + 1);
      if (link) {
        const label = format(parts.splice(open).slice(1).join(''), slots);
        put(keep(safeHref(link[0]) ? `<a href="${esc(link[0])}">${label}</a>` : label), link[1]);
      } else i++;
      open = -1;
    } else i++;
  }
  flush();
  return parts.join('');
}

// Underline, bold, italic and strikethrough around the placeholders of read(), and then the HTML they hold.
function format(s, slots) {
  const keep = (html) => `\u0000${slots.push(html) - 1}\u0000`;
  // Every <u> before the last </u> has one to close it, so each search ends at the nearest one.
  const last = s.lastIndexOf('</u>');
  if (last >= 0) s = s.slice(0, last + 4).replace(/<u>(.*?)<\/u>/gs, (_, c) => keep(`<u>${format(c, slots)}</u>`)) + s.slice(last + 4);
  s = esc(s).replace(/\*\*(\S[\s\S]*?)\*\*/g, '<b>$1</b>').replace(/\*(\S[^*]*?)\*/g, '<i>$1</i>').replace(/~~(\S.*?)~~/g, '<s>$1</s>');
  return s.replace(/\u0000(\d+)\u0000/g, (_, n) => slots[Number(n)]);
}

export function markdownToNotes(text) {
  if (!text) return '';
  // inline() marks its placeholders with NUL, so the Markdown can't bring one of its own.
  const lines = String(text || '').replace(/\u0000/g, '').replace(/\r\n?/g, '\n').split('\n');
  const out = [];
  const lists = [];
  const close = (indent = -1) => {
    while (lists.length && lists.at(-1).indent >= indent) out.push(`</li></${lists.pop().tag}>`);
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const li = /^(\s*)([-*+]|\d+\.)\s+(.*)$/.exec(line);
    if (li) {
      const indent = li[1].length;
      const tag = /^\d/.test(li[2]) ? 'ol' : 'ul';
      while (lists.length && lists.at(-1).indent > indent) close(lists.at(-1).indent);
      if (lists.at(-1)?.indent === indent && lists.at(-1).tag !== tag) close(indent);
      if (lists.at(-1)?.indent === indent) out.push('</li>');
      else { out.push(`<${tag}>`); lists.push({ indent, tag }); }
      out.push(`<li>${inline(li[3])}`);
      continue;
    }
    const indent = /^\s*/.exec(line)[0].length;
    if (lists.length && indent > lists[0].indent) {
      let closed = false;
      while (lists.length > 1 && indent <= lists.at(-1).indent) {
        close(lists.at(-1).indent);
        closed = true;
      }
      // Indented blank lines belong to the item too. Returning from a nested list
      // already starts a line, while an explicit blank continuation still needs BR.
      out.push(`${closed && line.trim() ? '' : '<br>'}${inline(line.trimStart())}`);
      continue;
    }
    close();
    const fence = /^(`{3,})/.exec(line);
    if (fence) {
      const code = [];
      for (i++; i < lines.length && lines[i].trim() !== fence[1]; i++) code.push(lines[i]);
      out.push(`<pre><code>${esc(code.join('\n'))}</code></pre>`);
    } else {
      const heading = /^(#{1,3})\s+(.*)$/.exec(line);
      if (heading) out.push(`<h${heading[1].length}>${inline(heading[2])}</h${heading[1].length}>`);
      else if (/^> ?/.test(line)) out.push(`<blockquote>${inline(line.replace(/^> ?/, ''))}</blockquote>`);
      else if (/^---$/.test(line)) out.push('<hr>');
      else out.push(`<div>${line ? inline(line) : '<br>'}</div>`);
    }
  }
  close();
  return sanitizeNotes(out.join(''));
}

const escapeMarkdown = (s) => s.replace(/([\\`*_\[\]<>~])/g, '\\$1').replace(/^(\s*)([#>+\-])/gm, '$1\\$2').replace(/^(\s*\d+)\./gm, '$1\\.');
const ticksFor = (text, min = 1) => '`'.repeat(Math.max(min, ...[...text.matchAll(/`+/g)].map((m) => m[0].length + 1)));

export function notesToMarkdown(editor) {
  // Parse inertly and strip controls and styles before serializing a paste.
  const doc = new DOMParser().parseFromString(sanitizeNotes(editor.innerHTML), 'text/html');
  const blocks = new Set(['DIV', 'P', 'H1', 'H2', 'H3', 'UL', 'OL', 'BLOCKQUOTE', 'PRE', 'HR']);
  const children = (el) => {
    let text = '';
    for (const child of el.childNodes) {
      const next = write(child);
      // Chromium can leave a bare text node before the DIV created by Enter. A block
      // starts a new line there too, without adding a blank line after an adjacent block.
      if (blocks.has(child.tagName) && text && !text.endsWith('\n')) text += '\n';
      text += next;
    }
    return text;
  };
  // The reader parses one Markdown line at a time. Reopen an inline span on each
  // line instead of saving delimiters that cross a Shift+Enter boundary.
  const formatLines = (text, format) => text.split('\n').map((line) => {
    const m = /^(\s*)([\s\S]*?)(\s*)$/.exec(line);
    return m[2] ? `${m[1]}${format(m[2])}${m[3]}` : line;
  }).join('\n');
  function write(node) {
    if (node.nodeType === Node.TEXT_NODE) return escapeMarkdown(node.textContent.replace(/\u00a0/g, ' '));
    if (node.nodeType !== Node.ELEMENT_NODE) return '';
    const body = () => children(node);
    const wrap = (mark) => formatLines(body(), (line) => `${mark}${line}${mark}`);
    switch (node.tagName) {
      case 'BR': return '\n';
      case 'B': case 'STRONG': return wrap('**');
      case 'I': case 'EM': return wrap('*');
      case 'S': return wrap('~~');
      case 'U': return formatLines(body(), (line) => `<u>${line}</u>`);
      case 'A': {
        const href = node.getAttribute('href') || '';
        return safeHref(href) ? formatLines(body(), (line) => `[${line}](<${href.replace(/>/g, '%3E').replace(/</g, '%3C')}>)`) : body();
      }
      case 'H1': case 'H2': case 'H3': return body().trim().split('\n').map((line) => line.trim() ? `${'#'.repeat(Number(node.tagName[1]))} ${line.trim()}` : '').join('\n') + '\n';
      case 'UL': case 'OL': return [...node.children].filter((n) => n.tagName === 'LI').map((li, i) => {
        const content = children(li).replace(/\n+$/, '');
        const lines = content.split('\n');
        return `${node.tagName === 'UL' ? '-' : `${i + 1}.`} ${lines[0]}\n${lines.slice(1).map((l) => `    ${l}\n`).join('')}`;
      }).join('');
      case 'BLOCKQUOTE': return body().trimEnd().split('\n').map((l) => `> ${l}\n`).join('');
      case 'PRE': {
        const fence = ticksFor(node.textContent, 3);
        return `${fence}\n${node.textContent}\n${fence}\n`;
      }
      case 'CODE': {
        return node.textContent.split('\n').map((text) => {
          if (!text) return '';
          const ticks = ticksFor(text);
          // The reader takes a space off both ends of code that has more than spaces (inline()).
          const pad = /^`|`$/.test(text) || (text.startsWith(' ') && text.endsWith(' ') && text.trim()) ? ' ' : '';
          return `${ticks}${pad}${text}${pad}${ticks}`;
        }).join('\n');
      }
      case 'HR': return '---\n';
      case 'DIV': case 'P': {
        const s = body();
        return s.endsWith('\n') ? s : `${s}\n`;
      }
      case 'LI': return body();
      default: return body();
    }
  }
  return children(doc.body).replace(/\n$/, '');
}
