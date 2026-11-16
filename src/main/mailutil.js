'use strict';

const libqp = require('libqp');
const iconv = require('iconv-lite');
const crypto = require('crypto');
const { convert } = require('html-to-text');
const { t } = require('../i18n');

const ROLE_NAMES = {
  get inbox() { return t('mailbox.folders.inbox'); },
  get sent() { return t('mailbox.folders.sent'); },
  get drafts() { return t('mailbox.folders.drafts'); },
  get trash() { return t('mailbox.folders.trash'); },
  get junk() { return t('mailbox.folders.junk'); },
  get archive() { return t('mailbox.folders.archive'); }
};

const SPECIAL_USE = {
  '\\Inbox': 'inbox',
  '\\Sent': 'sent',
  '\\Drafts': 'drafts',
  '\\Trash': 'trash',
  '\\Junk': 'junk',
  '\\Archive': 'archive',
  '\\All': 'all',
  '\\Flagged': 'flagged',
  '\\Important': 'important'
};

// Fallback for servers without SPECIAL-USE: match common names in English and Dutch.
const NAME_ROLES = [
  [/^(inbox|postvak in)$/i, 'inbox'],
  [/^(sent|sent items|sent mail|sent messages|verzonden|verzonden items)$/i, 'sent'],
  [/^(drafts|draft|concepten)$/i, 'drafts'],
  [/^(trash|deleted|deleted items|deleted messages|bin|prullenbak|verwijderde items)$/i, 'trash'],
  [/^(junk|spam|junk e-mail|junk email|bulk mail|ongewenste e-mail)$/i, 'junk'],
  [/^(archive|archives|archief)$/i, 'archive']
];

function folderRole(folder) {
  if (folder.path && folder.path.toUpperCase() === 'INBOX') return 'inbox';
  if (folder.specialUse && SPECIAL_USE[folder.specialUse]) return SPECIAL_USE[folder.specialUse];
  for (const flag of folder.flags || []) {
    if (SPECIAL_USE[flag]) return SPECIAL_USE[flag];
  }
  const leaf = String(folder.name || folder.path || '').trim();
  for (const [re, role] of NAME_ROLES) {
    if (re.test(leaf)) return role;
  }
  return null;
}

function displayFolderName(folder) {
  if (folder.role && ROLE_NAMES[folder.role]) return ROLE_NAMES[folder.role];
  return folder.name || folder.path;
}

// Walks an imapflow bodyStructure tree depth-first.
function walkStructure(node, visit) {
  if (!node) return;
  visit(node);
  for (const child of node.childNodes || []) walkStructure(child, visit);
}

function partKey(node) {
  // Single-part messages have no part number; IMAP addresses their body as part 1.
  return node.part || '1';
}

function isAttachmentNode(node) {
  const type = String(node.type || '').toLowerCase();
  if (type.startsWith('multipart/')) return false;
  if (node.disposition === 'attachment') return true;
  if (type === 'message/rfc822') return true;
  if (type.startsWith('text/') && node.disposition !== 'attachment') return false;
  // Inline images referenced by cid are part of the body, not attachments.
  if (node.disposition === 'inline' && node.id) return false;
  return Boolean(node.dispositionParameters?.filename || node.parameters?.name);
}

function findPreviewPart(structure) {
  let plain = null;
  let html = null;
  walkStructure(structure, (node) => {
    if (isAttachmentNode(node)) return;
    const type = String(node.type || '').toLowerCase();
    if (type === 'text/plain' && !plain) plain = node;
    if (type === 'text/html' && !html) html = node;
  });
  return plain || html;
}

function hasAttachments(structure) {
  let found = false;
  walkStructure(structure, (node) => {
    if (isAttachmentNode(node)) found = true;
  });
  return found;
}

// ---------- a large email's text before its attachments (imap.js fetchReadable) ----------

// How much of the pictures an email's text shows by Content-ID comes with the text. The rest shows once the whole
// email is in.
const INLINE_IMAGES = 2 * 1024 * 1024;
// More parts than this go in one command only with the whole email.
const MAX_PARTS = 100;
// The header that gives the size of a part skeletonSource left empty.
const OMITTED = 'X-Rukoo-Omitted';

const nodeType = (node) => String(node.type || '').toLowerCase();

// What mailparser shows as the email's text instead of listing it as an attachment: text and delivery reports that
// aren't attachments, and an email forwarded inline, whose text it shows too (mailsplit's message/rfc822 rule).
function readsAsText(node) {
  const type = nodeType(node);
  if (['text/plain', 'text/html', 'message/delivery-status'].includes(type)) return !node.disposition || node.disposition === 'inline';
  if (type === 'message/rfc822') return node.disposition === 'inline' && ['', '7bit', '8bit', 'binary'].includes(String(node.encoding || '').toLowerCase());
  return false;
}

// A calendar part, which the reader shows as an invitation card (calendar/invite.js calendarPart).
function isCalendarNode(node) {
  const name = String((node.dispositionParameters && node.dispositionParameters.filename) || (node.parameters && node.parameters.name) || '');
  return /^(text\/calendar|application\/ics|text\/x-vcalendar)$/.test(nodeType(node)) || /\.ics$/i.test(name);
}

// The parts of an email to fetch so the reader can show it before its attachments are in: the MIME header of every
// part, and the body of the parts that read as text, of calendar parts, and of the pictures the text shows by
// Content-ID while they fit in INLINE_IMAGES. Keys as imapflow's bodyParts takes them. size: the whole email's.
// null when splitting doesn't pay: an email of one part, one of very many parts, or one whose text is half of it or
// more.
function readableParts(structure, size) {
  if (!structure || !nodeType(structure).startsWith('multipart/') || !Array.isArray(structure.childNodes)) return null;
  const keys = [];
  let wanted = 0;
  let images = 0;
  const visit = (node) => {
    if (!node.part) throw new Error('A part without a number');
    keys.push(`${node.part}.MIME`);
    const type = nodeType(node);
    if (type.startsWith('multipart/')) return (node.childNodes || []).forEach(visit);
    const bytes = Number(node.size) || 0;
    let take = readsAsText(node) || isCalendarNode(node);
    if (!take && node.id && type.startsWith('image/') && images + bytes <= INLINE_IMAGES) {
      images += bytes;
      take = true;
    }
    if (take) {
      keys.push(node.part);
      wanted += bytes;
    }
  };
  structure.childNodes.forEach(visit);
  if (keys.length > MAX_PARTS || !size || wanted * 2 >= size) return null;
  return keys;
}

// About how many bytes a part holds once decoded: a line of 76 base64 characters and its CRLF hold 57.
function decodedSize(size, encoding) {
  const n = Number(size) || 0;
  return String(encoding || '').toLowerCase() === 'base64' ? Math.floor((n * 57) / 78) : n;
}

// A header without the empty line after it, its last line ending in CRLF.
function headerLines(buf) {
  let end = buf.length;
  while (end && (buf[end - 1] === 0x0a || buf[end - 1] === 0x0d)) end--;
  return Buffer.concat([buf.subarray(0, end), Buffer.from('\r\n')]);
}

// An email rebuilt from its header, its BODYSTRUCTURE and the parts readableParts names, as imapflow fetched them
// (parts: a Map from lower-case key to Buffer). Each part keeps its own MIME header and the email's own boundaries; a
// part that wasn't fetched is empty, with its decoded size in an X-Rukoo-Omitted header. mailparser reads it the way
// it reads the whole email, so the attachments come in the same order and keep their index. Throws when a part's
// header or a boundary is missing.
function skeletonSource(header, structure, parts) {
  const out = [headerLines(header), Buffer.from('\r\n')];
  const add = (s) => out.push(Buffer.from(s));
  const multipart = (node) => {
    const boundary = node.parameters && node.parameters.boundary;
    if (!boundary) throw new Error('A multipart part without a boundary');
    for (const child of node.childNodes || []) {
      const mime = parts.get(`${child.part}.mime`);
      if (!mime) throw new Error(`No header for part ${child.part}`);
      add(`--${boundary}\r\n`);
      out.push(headerLines(mime));
      const body = parts.get(String(child.part).toLowerCase());
      if (nodeType(child).startsWith('multipart/')) {
        add('\r\n');
        multipart(child);
      } else if (body) {
        add('\r\n');
        out.push(body);
      } else {
        add(`${OMITTED}: ${decodedSize(child.size, child.encoding)}\r\n\r\n`);
      }
      add('\r\n');
    }
    add(`--${boundary}--\r\n`);
  };
  multipart(structure);
  return Buffer.concat(out);
}

function decodeTransfer(buffer, encoding) {
  const enc = String(encoding || '').toLowerCase();
  if (enc === 'base64') {
    // Partial fetches can cut a base64 group in half; drop the incomplete tail.
    const clean = buffer.toString('ascii').replace(/[^A-Za-z0-9+/=]/g, '');
    return Buffer.from(clean.slice(0, clean.length - (clean.length % 4)), 'base64');
  }
  if (enc === 'quoted-printable') return libqp.decode(buffer.toString('latin1'));
  return buffer;
}

function decodeCharset(buffer, charset) {
  const cs = String(charset || 'utf-8').toLowerCase();
  try {
    if (iconv.encodingExists(cs)) return iconv.decode(buffer, cs);
  } catch (_) {
    // Unknown charset: fall through to utf-8.
  }
  return buffer.toString('utf8');
}

function htmlToPlain(html) {
  try {
    return convertHtml(html);
  } catch (_) {
    // A preview is never worth failing a sync over; fall back to stripping tags.
    return String(html || '')
      .replace(/<(style|script|head)[\s\S]*?<\/\1>/gi, ' ')
      .replace(/<[^>]+>/g, ' ')
      .replace(/&nbsp;/g, ' ');
  }
}

function convertHtml(html) {
  return convert(html, {
    wordwrap: false,
    selectors: [
      { selector: 'a', options: { ignoreHref: true } },
      { selector: 'img', format: 'skip' },
      { selector: 'style', format: 'skip' },
      { selector: 'script', format: 'skip' },
      { selector: 'head', format: 'skip' },
      ...['h1', 'h2', 'h3', 'h4', 'h5', 'h6'].map((h) => ({ selector: h, options: { uppercase: false } })),
      { selector: 'table', format: 'block' },
      // Table cells on lines of their own, so a signature's name and phone number don't run together.
      { selector: 'td', format: 'block', options: { leadingLineBreaks: 1, trailingLineBreaks: 1 } },
      { selector: 'th', format: 'block', options: { leadingLineBreaks: 1, trailingLineBreaks: 1 } }
    ]
  });
}

function makePreview(text, max = 200) {
  return String(text || '')
    .replace(/ |‌|​|͏|﻿/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, max);
}

function previewFromPart(buffer, node) {
  if (!buffer || !node) return '';
  const decoded = decodeCharset(decodeTransfer(buffer, node.encoding), node.parameters?.charset);
  const type = String(node.type || '').toLowerCase();
  // Truncated HTML may end mid-tag; html-to-text tolerates that.
  const text = type === 'text/html' ? htmlToPlain(decoded) : decoded;
  return makePreview(text);
}

function addressFromEnvelope(list) {
  return (list || []).map((a) => ({ name: a.name || '', address: a.address || '' }));
}

function addressFromParsed(field) {
  if (!field) return [];
  const values = Array.isArray(field) ? field.flatMap((f) => f.value || []) : field.value || [];
  const out = [];
  for (const v of values) {
    if (v.group) out.push(...v.group.map((g) => ({ name: g.name || '', address: g.address || '' })));
    else out.push({ name: v.name || '', address: v.address || '' });
  }
  return out;
}

// Removes active content from email HTML before it is shown in the sandboxed reader frame.
function sanitizeHtml(html) {
  return String(html || '')
    .replace(/<script[\s\S]*?<\/script\s*>/gi, '')
    .replace(/<\/?(iframe|object|embed|frame|frameset|applet|form|base)\b[^>]*>/gi, '')
    .replace(/<meta[^>]+http-equiv[^>]*>/gi, '')
    .replace(/\son[a-z]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi, '')
    .replace(/(href|src|action)\s*=\s*(["']?)\s*javascript:[^"'\s>]*\2/gi, '$1="#"');
}

// Pictures in the editor (a signature's logo, an inserted image) are inline data. Mail apps such as Gmail and
// Outlook drop data: images, so they go out as inline attachments the HTML refers to by Content-ID; on reading,
// mailparser turns those back into data. The same picture twice is attached once. (Nodemailer's attachDataUrls
// only works when sending through its transport, not for MailComposer, which builds Rukoo's messages.)
const DATA_IMAGE = /(<img\b[^>]*?\ssrc\s*=\s*)(["'])data:image\/(png|jpe?g|gif|webp);base64,([a-z0-9+/=\s]+)\2/gi;

function inlineDataImages(html) {
  const attachments = [];
  const byData = new Map();
  const out = String(html || '').replace(DATA_IMAGE, (_match, prefix, quote, type, data) => {
    const base64 = data.replace(/\s+/g, '');
    let cid = byData.get(base64);
    if (!cid) {
      cid = `${crypto.randomBytes(10).toString('hex')}@rukoo`;
      byData.set(base64, cid);
      const ext = type.toLowerCase() === 'jpg' ? 'jpeg' : type.toLowerCase();
      attachments.push({
        filename: `image-${attachments.length + 1}.${ext === 'jpeg' ? 'jpg' : ext}`,
        content: Buffer.from(base64, 'base64'),
        contentType: `image/${ext}`,
        cid,
        contentDisposition: 'inline'
      });
    }
    return `${prefix}${quote}cid:${cid}${quote}`;
  });
  return { html: out, attachments };
}

function escapeHtml(text) {
  return String(text || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function textToHtml(text) {
  const linked = escapeHtml(text).replace(
    /(https?:\/\/[^\s<]+[^\s<.,;:!?)\]'"])/g,
    '<a href="$1">$1</a>'
  );
  return `<div style="white-space:pre-wrap;word-wrap:break-word">${linked}</div>`;
}

// The List-Unsubscribe header (RFC 2369), with one-click support (RFC 8058) for https links.
function unsubscribeInfo(parsed) {
  const list = parsed && parsed.headers && parsed.headers.get('list');
  const u = list && list.unsubscribe;
  if (!u) return null;
  const url = u.url && /^https?:\/\//i.test(u.url) ? u.url : null;
  const mail = u.mail ? `mailto:${u.mail}` : null;
  const post = list['unsubscribe-post'];
  const oneClick = Boolean(url && /^https:/i.test(url) && post && /one-click/i.test(post.name || ''));
  return url || mail ? { url, mail, oneClick } : null;
}

// ---------- threads ----------

// A Message-ID as a comparable key: without its angle brackets, in lower case.
const normId = (v) => String(v || '').trim().replace(/^<|>$/g, '').toLowerCase();

// A subject without the Re: and Fwd: that replies and forwards add, in their Dutch and German forms too, for
// comparing two subjects.
function baseSubject(subject) {
  let s = String(subject || '').trim();
  const prefix = /^(?:re|fwd?|fw|antw|aw|doorst)(?:\s*\[\d+\])?\s*:\s*/i;
  for (let prev = null; prev !== s; ) {
    prev = s;
    s = s.replace(prefix, '').trim();
  }
  return s.toLowerCase().replace(/\s+/g, ' ');
}

// The Message-IDs an email names: its own, the one it answers, and its References.
function threadIds(m) {
  return new Set([m.messageId, m.inReplyTo, ...[].concat(m.references || [])].map(normId).filter(Boolean));
}

// Whether an email is linked by its headers to one that names these ids: it is one of them, it answers one of them,
// or its References name one. The folder cache has no References; an email a chat keeps can have them.
function linked(ids, r) {
  const mid = normId(r.messageId);
  if ((mid && ids.has(mid)) || (r.inReplyTo && ids.has(normId(r.inReplyTo)))) return true;
  return [].concat(r.references || []).some((v) => ids.has(normId(v)));
}

module.exports = {
  normId,
  baseSubject,
  threadIds,
  linked,
  ROLE_NAMES,
  unsubscribeInfo,
  folderRole,
  displayFolderName,
  walkStructure,
  partKey,
  isAttachmentNode,
  findPreviewPart,
  hasAttachments,
  INLINE_IMAGES,
  OMITTED,
  readableParts,
  decodedSize,
  skeletonSource,
  decodeTransfer,
  decodeCharset,
  htmlToPlain,
  makePreview,
  previewFromPart,
  addressFromEnvelope,
  addressFromParsed,
  sanitizeHtml,
  inlineDataImages,
  escapeHtml,
  textToHtml
};
