// Finding web addresses in text, for the links the editor makes while you paste or type (editor-links.js).
// No DOM in here, so node can test it (test/linkify.test.js).

// An address starts with https://, http:// or www., not in the middle of a word or of another address, and runs to
// the next space, angle bracket or double quote. Consume data: and blob: tokens whole so an address inside one
// is skipped even when its prefix ends with a comma or other punctuation.
const CANDIDATE = /(?<![\p{L}\p{N}@._/+=:-])(?:(?:data|blob):|(?<web>https?:\/\/|www\.))[^\s<>"“”«»]+/giu;
// Punctuation that ends a sentence rather than an address, in Latin and in Chinese or Japanese text.
const TRAILING = '.,;:!?\'"’‘”。、，．；：！？';
const CLOSERS = { ')': '(', ']': '[' };
const emphasisBefore = (text, start) => /(?:\*{1,3}|~~)$/.exec(text.slice(Math.max(0, start - 3), start))?.[0];

// The address without the punctuation after it: "(see www.example.com)." keeps www.example.com, while a closing
// bracket that has its opening one in the address stays. The brackets are counted once, so a long run of them after
// an address takes no longer than any other text.
function trimUrl(text, emphasis) {
  const n = { '(': 0, ')': 0, '[': 0, ']': 0 };
  for (let i = 0; i < text.length; i++) if (text[i] in n) n[text[i]]++;
  let end = text.length;
  while (end > 0) {
    const last = text[end - 1];
    if (TRAILING.includes(last)) end--;
    else if (CLOSERS[last] && n[last] > n[CLOSERS[last]]) {
      n[last]--;
      end--;
    } else if (emphasis && text.endsWith(emphasis, end)) {
      end -= emphasis.length;
      emphasis = undefined;
    } else break;
  }
  return text.slice(0, end);
}

// A site: a name, or an IPv6 address in brackets such as [::1].
function hasHost(text) {
  if (/^www\./i.test(text)) return /^www\.[\p{L}\p{N}-]+\.[\p{L}\p{N}]/iu.test(text);
  const authority = /^https?:\/\/([^/\\?#]+)/i.exec(text)?.[1] || '';
  if (authority.includes('[') || authority.includes(']')) return Boolean(URL.parse(text)?.hostname.startsWith('['));
  return /^[\p{L}\p{N}]/u.test(authority);
}

// The address a link to text gets: www.example.com opens over https.
const hrefFor = (text) => (/^www\./i.test(text) ? `https://${text}` : text);

// The addresses in text, in order: [{ start, end, text, href }].
export function findUrls(text) {
  const source = String(text || '');
  const found = [];
  for (const m of source.matchAll(CANDIDATE)) {
    if (!m.groups.web) continue;
    const url = trimUrl(m[0], emphasisBefore(source, m.index));
    if (hasHost(url)) found.push({ start: m.index, end: m.index + url.length, text: url, href: hrefFor(url) });
  }
  return found;
}

// The link address when text is one address and nothing else, as a clipboard holds after Copy link.
export function soleUrl(text) {
  const s = String(text || '').trim();
  const [url, ...more] = findUrls(s);
  return url && !more.length && url.start === 0 && url.end === s.length ? url.href : null;
}

// The address typed last in before, the text in front of the caret, once a space, Enter or Tab ends it:
// { start, end, text, href } within before, or null. Punctuation between the address and the caret stays outside.
export function typedUrl(before) {
  const s = String(before || '');
  // The last word, found from the end: a pattern such as /\S+$/ would try every start inside a long word.
  let start = s.length;
  while (start > 0 && !/\s/.test(s[start - 1])) start--;
  // Nothing typed since the last space.
  if (start === s.length) return null;
  const word = s.slice(start);
  const url = findUrls(word).pop();
  if (!url || trimUrl(word.slice(url.start), emphasisBefore(word, url.start)) !== url.text) return null;
  return { ...url, start: start + url.start, end: start + url.end };
}

const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

// A line of plain text as HTML that shows the same spaces: Chromium's paste keeps a run of spaces, a space at either
// end of a line and a tab this way too.
const lineText = (s) =>
  esc(s)
    .replace(/\t/g, '<span style="white-space:pre">\t</span>')
    .replace(/ {2,}/g, (run) => `${'&nbsp;'.repeat(run.length - 1)} `);

function lineHtml(line) {
  let html = '';
  let at = 0;
  for (const url of findUrls(line)) {
    html += `${lineText(line.slice(at, url.start))}<a href="${esc(url.href)}">${esc(url.text)}</a>`;
    at = url.end;
  }
  // A space at either end would not show, and the text around the paste would run into it.
  return (html + lineText(line.slice(at))).replace(/^ | $/g, '&nbsp;');
}

// Pasted plain text with its addresses as links, or null when it has none: one line as it is, several lines as the
// blocks Chromium's own paste makes. block: the tag of those blocks, 'li' inside a list.
export function linkedHtml(text, block = 'div') {
  const s = String(text || '').replace(/\r\n?/g, '\n');
  if (!findUrls(s).length) return null;
  const lines = s.split('\n');
  if (lines.length === 1) return lineHtml(s);
  return lines.map((line) => `<${block}>${line ? lineHtml(line) : '<br>'}</${block}>`).join('');
}

// What the Link field of the link popover means: an address with its scheme stays, an email address becomes a
// mailto: link and anything else opens over https.
export function linkHref(input) {
  const s = String(input || '').trim();
  if (!s) return '';
  if (/^(https?:|mailto:)/i.test(s)) return s;
  // An email address: one @, no space or slash, and a dot with something on both sides after the @. Two simple
  // tests, because one pattern for it backtracks over every dot and stalls on a long input.
  const email = /^[^\s@/]+@[^\s@/]+$/.test(s) && /.\../.test(s.slice(s.indexOf('@') + 1));
  return email ? `mailto:${s}` : `https://${s}`;
}

// An address as a browser reads it: without the spaces and control characters around it, and without tabs and line
// breaks inside it. A loop, because a pattern for the end would try again at every space of a long run inside.
export function bareHref(href) {
  const s = String(href || '').replace(/[\t\n\r]/g, '');
  let start = 0;
  let end = s.length;
  while (start < end && s.charCodeAt(start) <= 0x20) start++;
  while (end > start && s.charCodeAt(end - 1) <= 0x20) end--;
  return s.slice(start, end);
}

// The part of an address that says where it goes: the scheme and the host, with the user name and port when it has
// them, or the addresses of a mailto: link. Browsers read a backslash in a web address as a slash, so the host ends
// there too.
const SITE = /^(?:(https?:[/\\]*)([^/\\?#]*)|([a-z][a-z\d+.-]*:\/\/)([^/?#]*)|(mailto:)([^?#]*))/i;
// A longer host loses labels at its start. However long the site, the path keeps this many characters.
const HOST_MAX = 64;
const PATH_MIN = 16;

// text in max characters with its middle left out. head: the share of them that goes to the start.
function middle(text, max, head) {
  if (text.length <= max) return text;
  if (max < 2) return '…';
  const h = Math.floor((max - 1) * head);
  return `${text.slice(0, h)}…${text.slice(text.length - (max - 1 - h))}`;
}

// A host too long to show leaves out labels at its start, never the end that names the site.
function shortHost(host) {
  if (host.length <= HOST_MAX) return host;
  const labels = host.split('.');
  let kept = labels.splice(-2).join('.');
  while (labels.length && kept.length + labels.at(-1).length + 2 <= HOST_MAX) kept = `${labels.pop()}.${kept}`;
  return `…${kept}`;
}

// A long address as the link bubble shows it, as [site, path]. The site always shows its whole host, so
// accounts.google.com.attacker.example never reads as accounts.google.com; the path loses its middle and keeps its
// end, the name of the page or the file. It shows the address a browser would open, so a space in front can't hide
// the host from the scheme.
export function shortUrlParts(url, max = 44) {
  const s = bareHref(url);
  const m = SITE.exec(s);
  if (!m) return ['', middle(s, max, 0.62)];
  const scheme = m[1] ?? m[3] ?? m[5];
  const host = m[2] ?? m[4] ?? m[6];
  const rest = s.slice(m[0].length);
  if (s.length <= max) return [scheme + host, rest];
  const site = scheme + shortHost(host);
  return [site, middle(rest, Math.max(max - site.length, PATH_MIN), 0.25)];
}

export const shortenUrl = (url, max) => shortUrlParts(url, max).join('');
