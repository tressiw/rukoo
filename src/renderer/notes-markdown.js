// The notes editor supports a small Markdown vocabulary. DOM output is escaped before the
// shared sanitizer sees it; pasted HTML goes through the same serializer before it is saved.
import { sanitizeAgentHtml } from './agent/richtext.js';

// The store limits Markdown to 100,000 characters. Escaping and editor wrappers can
// expand valid notes beyond the mail HTML budget; sanitize them without clipping.
const sanitizeNotes = (html) => sanitizeAgentHtml(html, { maxInput: Infinity });

const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const safeHref = (s) => /^(https?:|mailto:)/i.test(s);

// slots: the HTML of each placeholder, sources: the Markdown it was read from.
function inline(text, slots = [], sources = []) {
  const keep = (html, source) => {
    sources.push(source);
    return `\u0000${slots.push(html) - 1}\u0000`;
  };
  let s = text.replace(/\\([\\`*_{}\[\]()#+.!<>~-])|(`+)([^\n]*?)\2(?!`)/g, (source, escaped, ticks, c) => {
    if (escaped) return keep(esc(escaped), source);
    if (c.startsWith(' ') && c.endsWith(' ') && c.trim()) c = c.slice(1, -1);
    return keep(`<code>${esc(c)}</code>`, source);
  });
  // The editor writes an address in angle brackets as it is (notesToMarkdown), so a backtick or a backslash in it is
  // no code span or escape: it gets back the Markdown those took.
  const written = (href) => href.replace(/\u0000(\d+)\u0000/g, (_, n) => sources[Number(n)]);
  s = s.replace(/\[([^\]\n]+)\]\(<([^>]+)>\)/g, (source, label, href) => {
    href = written(href);
    return safeHref(href) ? keep(`<a href="${esc(href)}">${inline(label, slots, sources)}</a>`, source) : keep(inline(label, slots, sources), source);
  });
  s = s.replace(/\[([^\]\n]+)\]\(([^\s)]+)\)/g, (source, label, href) => safeHref(href) ? keep(`<a href="${esc(href)}">${inline(label, slots, sources)}</a>`, source) : keep(inline(label, slots, sources), source));
  s = s.replace(/<u>(.*?)<\/u>/g, (source, c) => keep(`<u>${inline(c, slots, sources)}</u>`, source));
  s = esc(s).replace(/\*\*(\S[\s\S]*?)\*\*/g, '<b>$1</b>').replace(/\*(\S[^*]*?)\*/g, '<i>$1</i>').replace(/~~(\S.*?)~~/g, '<s>$1</s>');
  // Recursive labels share the table that owns their escaped text and code placeholders.
  for (let i = 0; i <= slots.length && s.includes('\u0000'); i++) s = s.replace(/\u0000(\d+)\u0000/g, (_, n) => slots[Number(n)]);
  return s;
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
          const pad = /^`|`$|^ .* $/.test(text) ? ' ' : '';
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
