import { t, localize, translate, getLanguage } from './i18n.js';
// The message editor. It runs inline in the main window's reading pane, or on its own in a
// compose window (compose.js). Either way it saves drafts as you write.
import { formatButtons, mountRichEditor } from './rich-editor.js';
import { soleImage } from './editor-images.js';
import { icons } from './icons.js';
import { api, esc, $, toast, dialog, confirmDialog, showMenu, menuOpen, person, formatAddress, quoteDate, fileSize, hhmm, isLight, KEEP_OPEN } from './ui.js';
import { fillFrame } from './mailframe.js';
import { editMenu } from './editmenu.js';
import { sanitizeAgentHtml, textToEditorHtml } from './agent/richtext.js';
import { FREE_LENGTHS, FREE_RANGES, pcZone, freeRange, zoneOptions, duration } from './calendar-shared.js';
import { mountMentions, outgoingHtml } from './mentions.js';

const EMAIL_RE = /^[^@\s,;<>]+@[^@\s,;<>]+\.[^@\s,;<>]+$/;
// Autosave this long after the last change.
const AUTOSAVE_MS = 8000;
export const COMPOSE_TITLES = { get new() { return t('composer.titles.new'); }, get reply() { return t('composer.titles.reply'); }, get replyAll() { return t('composer.titles.replyAll'); }, get forward() { return t('composer.titles.forward'); }, get draft() { return t('composer.titles.draft'); } };

let contactsCache = null;
let attachmentKey = 0;
const keyed = (a) => ({ ...a, key: ++attachmentKey });

function prefixed(subject, prefix) {
  const s = subject || '';
  const re = prefix === 'Re' ? /^(re|antw|aw)\s*:/i : /^(fwd?|doorst)\s*:/i;
  return re.test(s.trim()) ? s : `${prefix}: ${s}`;
}

function sameAddress(a, b) {
  return String(a || '').toLowerCase() === String(b || '').toLowerCase();
}

function uniq(list) {
  const seen = new Set();
  return list.filter((a) => {
    const k = String(a.address || '').toLowerCase();
    if (!k || seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}

// Recipients from an agent: { name, address } objects or "Name <address>" strings.
function toAddress(a) {
  if (typeof a === 'string') {
    const m = a.match(/^(.*)<([^>]+)>\s*$/);
    return m ? { name: m[1].trim().replace(/^"|"$/g, ''), address: m[2].trim() } : { name: '', address: a.trim() };
  }
  return { name: String((a && a.name) || '').slice(0, 200), address: String((a && a.address) || '').trim().slice(0, 320) };
}

const addressList = (list) => (Array.isArray(list) ? uniq(list.slice(0, 100).map(toAddress)) : []);

// A reopened draft goes into this document itself, so it is parsed in an inert
// document and reduced to plain content: no style sheets, scripts, form controls,
// app class names, data attributes (the composer acts on those) or fixed positioning
// that could cover the editor. The signature and @-mentions keep their class.
function cleanDraftHtml(html) {
  const doc = new DOMParser().parseFromString(String(html || ''), 'text/html');
  doc.querySelectorAll('style, link, meta, script, noscript, iframe, frame, object, embed, base, form, title, template, button, input, select, textarea').forEach((n) => n.remove());
  for (const el of doc.body.querySelectorAll('*')) {
    for (const attr of [...el.attributes]) {
      const name = attr.name.toLowerCase();
      if (name.startsWith('on') || name.startsWith('data-') || name === 'id' || name === 'contenteditable' || name === 'tabindex') el.removeAttribute(attr.name);
      else if (name === 'class' && attr.value !== 'signature' && attr.value !== 'mention') el.removeAttribute(attr.name);
      else if ((name === 'href' || name === 'src') && /^\s*(javascript|vbscript|data:text\/html)/i.test(attr.value)) el.removeAttribute(attr.name);
    }
    if (el.style && /fixed|sticky|absolute/i.test(el.style.position)) el.style.position = '';
  }
  // The font wrapper payload() puts around the text (and Android's, the same) comes off again, also when earlier
  // saves nested it: the text and the signature are the editor's own lines, as in a new message.
  for (let wrap = ownWrapper(doc.body); wrap; wrap = ownWrapper(doc.body)) wrap.replaceWith(...wrap.childNodes);
  return doc.body.innerHTML;
}

// The body's one element when it is that wrapper, with nothing else around it but white space. Exactly its font
// and size: a block with any other style of its own is the user's formatting.
function ownWrapper(body) {
  const el = body.firstElementChild;
  if (!el || body.childElementCount !== 1 || el.tagName !== 'DIV' || el.attributes.length !== 1 || el.style.length !== 2) return null;
  if ([...body.childNodes].some((n) => n !== el && n.textContent.trim())) return null;
  return el.style.fontFamily.replace(/["'\s]/g, '') === 'SegoeUI,Arial,sans-serif' && el.style.fontSize === '12pt' ? el : null;
}

// A saved reply carries the earlier mail after its own text: a header block, then a blockquote.
// Split that off again, so a reopened draft shows it behind the "···" pill instead of in the text.
function splitQuote(html) {
  const doc = new DOMParser().parseFromString(String(html || ''), 'text/html');
  for (const head of doc.body.querySelectorAll('div')) {
    // New drafts carry a stable marker; recognize headers in earlier English/Dutch drafts too.
    if (!head.hasAttribute('data-rukoo-quote') && !/^-------- (Oorspronkelijk bericht|Original message) --------/.test(head.textContent.trim())) continue;
    let quote = head.nextElementSibling;
    while (quote && quote.tagName === 'BR') quote = quote.nextElementSibling;
    if (!quote || quote.tagName !== 'BLOCKQUOTE') continue;
    head.querySelectorAll('br').forEach((br) => br.replaceWith('\n'));
    const text = head.textContent.trim();
    const quoteHtml = quote.innerHTML;
    // Remove the line break before the header, the header, the breaks after it and the quote.
    if (head.previousSibling && head.previousSibling.nodeName === 'BR') head.previousSibling.remove();
    let node = head;
    while (node && node !== quote) {
      const next = node.nextSibling;
      node.remove();
      node = next;
    }
    quote.remove();
    return { body: doc.body.innerHTML, quoted: { head: text, html: quoteHtml } };
  }
  return { body: html, quoted: null };
}

// A signature as HTML: the formatted one, or plain text the way the composer has always shown it.
export function signatureHtml(text, html) {
  return html || (text ? esc(text).replace(/\n/g, '<br>') : '');
}

// An account without a signature of its own (both null) uses the general one.
export const usesGeneralSignature = (acc) => acc.signature == null && !acc.signatureHtml;

export function accountSignatureHtml(acc, settings) {
  return usesGeneralSignature(acc) ? signatureHtml(settings.signature, settings.signatureHtml) : signatureHtml(acc.signature, acc.signatureHtml);
}

// The booking page's address as main reports it, when it is safe to put in a mail: https, or http on this computer
// for a local development server. Anything else (a javascript: link from a hostile server) is left out.
export function safeBookingUrl(url) {
  if (typeof url !== 'string' || url.length > 300 || /[\s"'<>]/.test(url)) return null;
  try {
    const u = new URL(url);
    if (u.protocol === 'https:' || (u.protocol === 'http:' && ['127.0.0.1', 'localhost', '[::1]'].includes(u.hostname))) return url;
  } catch (_) {
    // Not an address.
  }
  return null;
}

// Text with {url} where the link goes, as HTML: the text escaped, the link's text the address itself, so the mail's
// plain-text part (which drops link targets) still carries it.
function withLink(text, url) {
  const link = `<a href="${esc(url)}">${esc(url)}</a>`;
  return String(text).split('{url}').map(esc).join(link);
}

// The booking link under the signature (contracts f): on in the account, or in the general settings when the account
// follows them (null), and only while the booking page is on. Its text is the account's, else the general one, else
// the default in the booking page's language. data: main's state, with bookingUrl and bookingLocale.
export function bookingFooterHtml(acc, data) {
  const url = safeBookingUrl(data && data.bookingUrl);
  if (!url) return '';
  const s = (data && data.settings) || {};
  const own = Boolean(acc) && typeof acc.bookingFooter === 'boolean';
  if (!(own ? acc.bookingFooter : s.bookingFooter === true)) return '';
  const fits = (v) => typeof v === 'string' && v.includes('{url}');
  const custom = own && fits(acc.bookingFooterText) ? acc.bookingFooterText : fits(s.bookingFooterText) ? s.bookingFooterText : null;
  // The default text's {url} stays for withLink(): translate() fills in each placeholder once, so it comes out as written.
  const text = custom || translate(data.bookingLocale || getLanguage(), 'composer.booking.footerDefault', { url: '{url}' });
  return `<div>${withLink(text, url)}</div>`;
}

// What goes in the .signature div of a new message: the signature, then the booking link. The div holds the footer
// also without a signature, so drafts and agents find it (cleanDraftHtml keeps only that class).
export function signatureBlockHtml(acc, data) {
  const signature = accountSignatureHtml(acc, data.settings);
  const footer = bookingFooterHtml(acc, data);
  if (!footer) return signature;
  return signature ? `${signature}${footer.replace('<div>', '<div style="margin-top:12px">')}` : footer;
}

// Cc, Bcc and a rich body can be seeded too (by an agent); they come on top of what the mode filled in.
function initialState(data, opts, m) {
  const s = modeState(data, opts, m);
  for (const [field, show] of [['cc', 'showCc'], ['bcc', 'showBcc']]) {
    const list = addressList(opts[field]);
    if (!list.length) continue;
    s[field] = list;
    s[show] = true;
  }
  return s;
}

// Whether a new message opens with something in it besides its recipients: a subject, text or Cc/Bcc from a
// mailto: link or an agent.
function prefilled(st, opts) {
  if (st.mode !== 'new') return false;
  const text = opts.html != null ? new DOMParser().parseFromString(sanitizeAgentHtml(opts.html), 'text/html').body.textContent : opts.body;
  return Boolean(st.subject.trim() || st.cc.length || st.bcc.length || String(text || '').trim());
}

function modeState(data, opts, m) {
  const accounts = data.accounts;
  const defaultAcc = accounts.find((a) => a.isDefault) || accounts[0];
  const acc = (m && accounts.find((a) => a.id === m.accountId)) || (opts.accountId && accounts.find((a) => a.id === opts.accountId)) || defaultAcc;
  const s = {
    mode: opts.mode,
    accountId: acc.id,
    from: acc.defaultFrom,
    to: opts.to ? [...opts.to] : [],
    cc: [],
    bcc: [],
    showCc: false,
    showBcc: false,
    subject: opts.subject || '',
    attachments: [],
    include: true,
    quoteOpen: false,
    original: null,
    // The earlier mail of a reopened reply: { head, html }.
    quoted: null,
    inReplyTo: null,
    references: [],
    draftId: null,
    replyToId: null,
    forwardId: null,
    bodyHtml: null,
    dirty: false,
    savedAt: null
  };
  // Stored signatures are cleaned by the main process (signature.js there); the booking link comes after it.
  const signature = signatureBlockHtml(acc, data);
  const sigHtml = signature ? `<div><br></div><div class="signature">${signature}</div>` : '';
  const body = opts.html != null ? sanitizeAgentHtml(opts.html) || '<div><br></div>' : `<div>${esc(opts.body || '').replace(/\n/g, '<br>') || '<br>'}</div>`;
  s.bodyHtml = `${body}${sigHtml}`;

  if (!m) return s;
  const own = (address) => acc.identities.some((i) => sameAddress(i.address, address));
  if (opts.mode === 'draft') {
    if (m.from && own(m.from.address)) s.from = acc.identities.find((i) => sameAddress(i.address, m.from.address)).address;
    s.to = m.to || [];
    s.cc = m.cc || [];
    s.bcc = m.bcc || [];
    s.showCc = s.cc.length > 0;
    s.showBcc = s.bcc.length > 0;
    s.subject = m.subject || '';
    // The quote is mail the engine already sanitized for the sandboxed frame; only the text goes into the editor.
    const parts = splitQuote(m.html);
    s.bodyHtml = cleanDraftHtml(parts.body);
    s.quoted = parts.quoted;
    s.draftId = m.id;
    s.savedAt = m.date || null;
    // Attachments stay on the stored draft; they are copied from it when sending or re-saving.
    s.forwardId = m.id;
    s.attachments = (m.attachments || []).map((a) => keyed({ forwardIndex: a.index, filename: a.filename, size: a.size }));
    s.inReplyTo = m.inReplyTo || null;
    s.references = m.references || [];
    return s;
  }
  s.original = m;
  if (opts.mode === 'reply' || opts.mode === 'replyAll') {
    const envelope = replyEnvelope(acc, m, opts.mode);
    s.to = envelope.to;
    if (opts.mode === 'replyAll') {
      s.cc = envelope.cc;
      s.showCc = s.cc.length > 0;
    }
    s.subject = prefixed(m.subject, 'Re');
    s.from = replyFrom(acc, m) || s.from;
    s.inReplyTo = m.messageId;
    s.references = [...(m.references || []), m.messageId].filter(Boolean);
    s.replyToId = m.id;
  }
  if (opts.mode === 'forward') {
    s.subject = prefixed(m.subject, 'Fwd');
    s.from = replyFrom(acc, m) || s.from;
    s.forwardId = m.id;
    s.attachments = (m.attachments || []).map((a) => keyed({ forwardIndex: a.index, filename: a.filename, size: a.size }));
  }
  return s;
}

// To and Cc of a reply to m: the sender (or its Reply-To), and for reply-all everyone else except you.
// Mail you sent yourself is answered to its original recipients.
export function replyEnvelope(acc, m, mode) {
  const own = (address) => acc.identities.some((i) => sameAddress(i.address, address));
  const fromSelf = own(m.from && m.from.address);
  let to = fromSelf ? [...(m.to || [])] : (m.replyTo && m.replyTo.length ? m.replyTo : [m.from]).filter(Boolean);
  let cc = [];
  if (mode === 'replyAll') {
    const extra = fromSelf ? [] : (m.to || []).filter((a) => !own(a.address));
    to = uniq([...to, ...extra]);
    cc = uniq((m.cc || []).filter((a) => !own(a.address) && !to.some((x) => sameAddress(x.address, a.address))));
  }
  return { to, cc };
}

// Answer from the address the mail was sent to, so mail to an alias is answered from that alias.
function replyFrom(acc, m) {
  if (m.from && acc.identities.some((i) => sameAddress(i.address, m.from.address))) {
    return acc.identities.find((i) => sameAddress(i.address, m.from.address)).address;
  }
  const recipients = [...(m.to || []), ...(m.cc || [])];
  const hit = acc.identities.find((i) => !i.primary && recipients.some((r) => sameAddress(r.address, i.address)));
  if (hit) return hit.address;
  const primary = recipients.some((r) => sameAddress(r.address, acc.email));
  return primary ? acc.email : null;
}

function quoteHeader(m) {
  const lines = [
    t('composer.quote.marker'),
    t('composer.quote.from', { sender: formatAddress(m.from) }),
    t('composer.quote.date', { date: quoteDate(m.date) }),
    t('composer.quote.to', { recipients: (m.to || []).map(formatAddress).join(', ') })
  ];
  if (m.cc && m.cc.length) lines.push(`Cc: ${m.cc.map(formatAddress).join(', ')}`);
  lines.push(t('composer.quote.subject', { subject: m.subject || '' }));
  return lines.join('\n');
}

function recipientsHtml(list) {
  return list
    .map(
      (a, i) =>
        `<span class="recipient ${EMAIL_RE.test(a.address) ? '' : 'invalid'}" title="${esc(formatAddress(a))}">${esc(person(a))}<button data-remove="${i}" data-i18n-title="common.actions.delete" title="${esc(t('common.actions.delete'))}" tabindex="-1">${icons.close}</button></span>`
    )
    .join('');
}

export { formatButtons } from './rich-editor.js';

function template(data, st, inline) {
  const accounts = data.accounts;
  // One entry per sending address; with several accounts the addresses are grouped per account.
  const fromOptions = (a) =>
    a.identities
      .map((i) => {
        const key = `${a.id}|${i.address}`;
        const on = a.id === st.accountId && sameAddress(i.address, st.from);
        return `<option value="${esc(key)}" ${on ? 'selected' : ''}>${esc(i.name ? `${i.name} <${i.address}>` : i.address)}</option>`;
      })
      .join('');
  const identityCount = accounts.reduce((n, a) => n + a.identities.length, 0);
  const acc = accounts.find((a) => a.id === st.accountId);
  const me = acc.identities.find((i) => sameAddress(i.address, st.from)) || { address: st.from, name: acc.name };
  const from =
    identityCount > 1
      ? `<select id="from" data-field="account">${
          accounts.length > 1 ? accounts.map((a) => `<optgroup label="${esc(a.email)}">${fromOptions(a)}</optgroup>`).join('') : fromOptions(accounts[0])
        }</select>`
      : `<span class="from-static">${esc(me.name ? `${me.name} <${me.address}>` : me.address)}</span>`;
  // New mail starts from the default account. A new message from another account offers to make that one the
  // default, beside From like Cc and Bcc beside To. mountComposer() shows it only for an account that isn't.
  const makeDefault =
    st.mode === 'new' && accounts.length > 1
      ? `<span class="cc-links from-default" data-default-link hidden><button class="link-btn" data-c="make-default" data-i18n="composer.actions.makeDefault">${esc(t('composer.actions.makeDefault'))}</button></span>`
      : '';
  const quoteHead = st.original ? quoteHeader(st.original) : st.quoted ? st.quoted.head : '';
  const quote = st.original || st.quoted
    ? `<div class="quote-zone">
        <button class="quote-pill" data-c="quote" data-i18n-title="composer.quote.show" title="${esc(t('composer.quote.show'))}" aria-expanded="false">···</button>
        <div class="quote" hidden>
          <div class="quote-bar"><span data-i18n="composer.quote.title">${esc(t('composer.quote.title'))}</span><button class="link-btn" data-c="include" data-i18n="composer.quote.exclude">${esc(t('composer.quote.exclude'))}</button></div>
          <div class="quote-head">${esc(quoteHead)}</div>
          <iframe class="quote-frame" sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox" data-i18n-title="composer.quote.title" title="${esc(t('composer.quote.title'))}"></iframe>
        </div>
      </div>`
    : '';
  return `
    ${
      inline
        ? `<div class="composer-head">
            <span class="composer-title">${esc(COMPOSE_TITLES[st.mode] || t('composer.titles.new'))}</span>
            <span class="spacer"></span>
            <button class="icon-btn" data-c="popout" data-i18n-title="composer.actions.popout" title="${esc(t('composer.actions.popout'))}">${icons.popOut}</button>
            <button class="icon-btn" data-c="close" data-i18n-title="composer.actions.closeShortcut" title="${esc(t('composer.actions.closeShortcut'))}">${icons.close}</button>
          </div>`
        : ''
    }
    <div class="composer-scroll">
      <div class="compose-fields">
        <div class="field from-field"><label for="from" data-i18n="reader.headers.from">${esc(t('reader.headers.from'))}</label>${from}${makeDefault}</div>
        <div class="field" data-rfield="to"><label data-i18n="reader.headers.to">${esc(t('reader.headers.to'))}</label><div class="recipients"></div>
          <span class="cc-links"><button class="link-btn" data-c="cc" data-i18n="reader.headers.cc" ${st.showCc ? 'hidden' : ''}>${esc(t('reader.headers.cc'))}</button><button class="link-btn" data-c="bcc" data-i18n="reader.headers.bcc" ${st.showBcc ? 'hidden' : ''}>${esc(t('reader.headers.bcc'))}</button></span></div>
        <div class="field" data-rfield="cc" ${st.showCc ? '' : 'hidden'}><label data-i18n="reader.headers.cc">${esc(t('reader.headers.cc'))}</label><div class="recipients"></div></div>
        <div class="field" data-rfield="bcc" ${st.showBcc ? '' : 'hidden'}><label data-i18n="reader.headers.bcc">${esc(t('reader.headers.bcc'))}</label><div class="recipients"></div></div>
        <div class="field"><label for="subject" data-i18n="reader.headers.subject">${esc(t('reader.headers.subject'))}</label><input id="subject" data-field="subject" type="text" value="${esc(st.subject)}" autocomplete="off"/></div>
      </div>
      <div class="compose-body">
        <div class="editor" contenteditable="true" spellcheck="true" role="textbox" aria-multiline="true" data-i18n-aria-label="composer.body.label" aria-label="${esc(t('composer.body.label'))}" data-i18n-data-placeholder="composer.body.placeholder" data-placeholder="${esc(t('composer.body.placeholder'))}"></div>
        ${quote}
      </div>
      <div class="compose-attachments" data-i18n-aria-label="mailbox.filters.attachments" aria-label="${esc(t('mailbox.filters.attachments'))}"></div>
    </div>
    <div class="format-row" role="toolbar" data-i18n-aria-label="composer.format.label" aria-label="${esc(t('composer.format.label'))}" hidden>
      ${formatButtons()}
      <button class="icon-btn sm" data-cmd="outdent" data-i18n-title="composer.format.outdent" title="${esc(t('composer.format.outdent'))}">${icons.outdent}</button>
      <button class="icon-btn sm" data-cmd="indent" data-i18n-title="composer.format.indent" title="${esc(t('composer.format.indent'))}">${icons.indent}</button>
      <span class="sep"></span>
      <button class="icon-btn sm" data-color="foreColor" data-i18n-title="composer.format.textColor" title="${esc(t('composer.format.textColor'))}">${icons.textColor}</button>
      <button class="icon-btn sm" data-color="hiliteColor" data-i18n-title="composer.format.highlight" title="${esc(t('composer.format.highlight'))}">${icons.highlight}</button>
      <button class="icon-btn sm" data-cmd="removeFormat" data-i18n-title="composer.format.clear" title="${esc(t('composer.format.clear'))}">${icons.clearFormat}</button>
    </div>
    <div class="composer-bar">
      <button class="icon-btn" data-c="attach" data-i18n-title="composer.attachments.pick" title="${esc(t('composer.attachments.pick'))}">${icons.clip}</button>
      <button class="icon-btn" data-c="image" data-i18n-title="composer.attachments.image" title="${esc(t('composer.attachments.image'))}">${icons.image}</button>
      <button class="icon-btn" data-c="calendar" data-i18n-title="composer.booking.button" title="${esc(t('composer.booking.button'))}" aria-haspopup="menu">${icons.calendar}</button>
      <button class="icon-btn" data-c="format" data-i18n-title="composer.format.label" title="${esc(t('composer.format.label'))}" aria-pressed="false">${icons.textFormat}</button>
      <button class="icon-btn" data-c="discard" data-i18n-title="composer.actions.discard" title="${esc(t('composer.actions.discard'))}">${icons.trash}</button>
      <button class="icon-btn" data-c="more" data-i18n-title="common.actions.moreOptions" title="${esc(t('common.actions.moreOptions'))}" aria-haspopup="menu">${icons.more}</button>
      <span class="spacer"></span>
      <span class="agent-mark" hidden>${icons.sparkle}<span class="agent-mark-text"></span><button class="link-btn" data-c="agent-undo" data-i18n="composer.agent.undo">${esc(t('composer.agent.undo'))}</button></span>
      <span class="save-state" aria-live="polite"></span>
      <button class="btn primary send-btn" data-c="send" data-i18n-title="composer.actions.sendShortcut" title="${esc(t('composer.actions.sendShortcut'))}"><span data-i18n="composer.actions.send">${esc(t('composer.actions.send'))}</span>${icons.send}</button>
    </div>
    <div class="float-bar" role="toolbar" data-i18n-aria-label="composer.format.label" aria-label="${esc(t('composer.format.label'))}" hidden>${formatButtons()}</div>
    <input type="color" class="color-input" value="#d6336c" tabindex="-1" aria-hidden="true"/>`;
}

// host: the element to render in. Returns a controller the main window uses to leave or close it.
// options: { data, opts, message, inline, onDone(), onPopOut(opts), onTitle(text) }
export function mountComposer(host, { data, opts, message, inline, onDone, onPopOut, onTitle }) {
  const st = initialState(data, opts, message);
  // Stable for the life of this composer: the chat panel ties an agent's draft card to it.
  const key = crypto.randomUUID();
  // Set once the draft is deleted or sent, so nothing points at a stored draft that is gone.
  let gone = false;
  const page = document.createElement('div');
  page.className = `compose composer ${inline ? 'inline' : ''}`;
  page.setAttribute('role', inline ? 'region' : 'main');
  page.setAttribute('aria-label', COMPOSE_TITLES[st.mode] || t('composer.titles.new'));
  page.innerHTML = template(data, st, inline);
  host.appendChild(page);
  // Listeners outside the composer, removed when it closes.
  const cleanups = [];
  const listen = (target, type, fn, opt) => {
    target.addEventListener(type, fn, opt);
    cleanups.push(() => target.removeEventListener(type, fn, opt));
  };
  const keyTarget = inline ? page : document;

  const editor = $('.editor', page);
  editor.innerHTML = st.bodyHtml;
  document.execCommand('styleWithCSS', false, true);

  const title = () => onTitle && onTitle(st.subject.trim() || COMPOSE_TITLES[st.mode] || t('composer.titles.new'));
  title();

  const quoteFrame = $('.quote-frame', page);
  let quoteDrawn = false;
  const drawQuote = () => {
    if (!quoteFrame || !st.quoteOpen) return;
    quoteDrawn = true;
    fillFrame(quoteFrame, st.original || { html: st.quoted.html, isHtml: true }, { light: isLight(), darkEmails: data.settings.darkEmails !== false });
  };

  // ----- save status -----

  const saveState = $('.save-state', page);
  const showSaveState = (text, tip = '') => {
    saveState.textContent = text;
    saveState.title = tip;
  };
  // Quiet while you write; only a saved draft is worth mentioning.
  const describeSaved = () => {
    page.dataset.dirty = String(st.dirty);
    if (st.saving) return showSaveState(t('common.status.saving'));
    if (!st.dirty && st.savedAt && st.draftId) return showSaveState(t('composer.status.saved'), t('composer.status.savedAt', { time: hhmm(st.savedAt) }));
    showSaveState('');
  };
  describeSaved();

  const renderRecipients = (field) => {
    const box = $(`[data-rfield="${field}"] .recipients`, page);
    const draft = box.querySelector('input')?.value || '';
    const labelKey = `reader.headers.${field}`;
    box.innerHTML = `${recipientsHtml(st[field])}<input type="text" data-rinput="${field}" autocomplete="off" spellcheck="false" data-i18n-aria-label="${labelKey}" aria-label="${esc(t(labelKey))}"/>`;
    const input = box.querySelector('input');
    input.value = draft;
    return input;
  };
  ['to', 'cc', 'bcc'].forEach(renderRecipients);

  const renderAttachments = () => {
    $('.compose-attachments', page).innerHTML = st.attachments
      .map(
        (a, i) =>
          `<span class="att-chip" title="${esc(a.path || a.filename)}">${icons.clip}<span class="n">${esc(a.filename)}</span>${a.size ? `<span class="s">${fileSize(a.size)}</span>` : ''}<button data-unattach="${i}" data-i18n-title="common.actions.delete" title="${esc(t('common.actions.delete'))}">${icons.close}</button></span>`
      )
      .join('');
  };
  renderAttachments();

  // ----- default account -----

  // The link beside From, on new messages with several accounts: shown while the chosen account isn't the default.
  const defaultLink = $('[data-default-link]', page);
  let defaultId = null;
  const showDefaultLink = () => {
    if (defaultLink) defaultLink.hidden = st.accountId === defaultId;
  };
  // Main's list of accounts; the default can change in Settings or in another composer while this one is open.
  const syncDefault = (accounts) => {
    defaultId = (accounts.find((a) => a.isDefault) || accounts[0] || {}).id || null;
    showDefaultLink();
  };
  syncDefault(data.accounts);

  // As Settings > Accounts does. It changes no part of the message, so the draft stays as it was.
  let makingDefault = false;
  async function makeDefault(b) {
    if (makingDefault) return;
    makingDefault = true;
    const id = st.accountId;
    try {
      await api('updateAccount', id, { makeDefault: true });
      defaultId = id;
      toast(t('composer.status.defaultAccount', { address: data.accounts.find((a) => a.id === id)?.email || '' }));
    } catch (err) {
      toast(err.message, 5000);
    } finally {
      makingDefault = false;
    }
    // The link makes way; the keyboard goes back to From.
    const refocus = document.activeElement === b;
    showDefaultLink();
    if (refocus && defaultLink.hidden) $('#from', page).focus();
  }

  // ----- dirty tracking and autosave -----

  let autosaveTimer = null;
  let autosave = true;
  // Counts edits, so a save that started before the latest edit leaves the window dirty.
  let editCount = 0;
  const armAutosave = () => {
    clearTimeout(autosaveTimer);
    if (autosave) autosaveTimer = setTimeout(() => saveDraft({ auto: true }), AUTOSAVE_MS);
  };

  // "Drafted by Clark" while an agent's text is in the editor, "· edited" once you change it.
  // st.agent: { name, conversationId, edited } or null. agentBefore: the content its Undo link restores;
  // covered: the agent writes (ids from the chat panel) that one Undo takes back.
  st.agent = null;
  let agentBefore = null;
  let covered = new Set();
  // Changes of From to another account, so Undo knows whether the signature in agentBefore is still the right one.
  let accountSwitches = 0;
  const agentMark = $('.agent-mark', page);
  const describeAgent = () => {
    agentMark.hidden = !st.agent;
    if (!st.agent) return;
    const text = t(st.agent.edited ? 'composer.agent.draftedEdited' : 'composer.agent.drafted', { name: st.agent.name });
    $('.agent-mark-text', page).textContent = text;
    agentMark.title = text;
    $('[data-c="agent-undo"]', page).hidden = !agentBefore;
  };

  // by: { name } when an agent made the change; anything else is the user's own edit.
  const changed = ({ by } = {}) => {
    editCount++;
    st.dirty = true;
    if (by) st.agent = { name: String(by.name || '').trim().slice(0, 60) || t('composer.agent.someone'), conversationId: by.conversationId ? String(by.conversationId) : null, edited: false };
    else {
      // Counted apart from dirty: autosave clears dirty, but the words are still the user's.
      st.userEdits = (st.userEdits || 0) + 1;
      if (st.agent) st.agent.edited = true;
    }
    describeAgent();
    describeSaved();
    armAutosave();
  };

  const commit = (field, text) => {
    const parts = String(text || '')
      .split(/[,;\n]+/)
      .map((s) => s.trim())
      .filter(Boolean);
    for (const p of parts) {
      const m = p.match(/^(.*)<([^>]+)>$/);
      const entry = m ? { name: m[1].trim().replace(/^"|"$/g, ''), address: m[2].trim() } : { name: '', address: p };
      if (!st[field].some((a) => sameAddress(a.address, entry.address))) st[field].push(entry);
    }
    if (parts.length) changed();
  };

  // Autocomplete from addresses seen in synced mail.
  let suggest = null;
  const closeSuggest = () => {
    suggest?.remove();
    suggest = null;
  };
  const showSuggest = async (input) => {
    const q = input.value.trim().toLowerCase();
    closeSuggest();
    if (q.length < 1) return;
    if (!contactsCache) contactsCache = await api('contacts').catch(() => []);
    const hits = contactsCache.filter((c) => `${c.name} ${c.address}`.toLowerCase().includes(q)).slice(0, 6);
    if (!hits.length || input.value.trim().toLowerCase() !== q) return;
    suggest = document.createElement('div');
    suggest.className = 'suggest';
    suggest.setAttribute('role', 'listbox');
    suggest.innerHTML = hits
      .map((c, i) => `<button data-i="${i}" role="option" class="${i === 0 ? 'active' : ''}"><div>${esc(c.name || c.address)}</div><div class="addr">${esc(c.address)}</div></button>`)
      .join('');
    suggest.hits = hits;
    suggest.addEventListener('mousedown', (e) => {
      const b = e.target.closest('button');
      if (!b) return;
      e.preventDefault();
      pick(hits[Number(b.dataset.i)], input);
    });
    input.closest('.field').appendChild(suggest);
  };
  const pick = (contact, input) => {
    const field = input.dataset.rinput;
    if (!st[field].some((a) => sameAddress(a.address, contact.address))) st[field].push({ ...contact });
    input.value = '';
    changed();
    closeSuggest();
    renderRecipients(field).focus();
  };

  page.addEventListener('input', (e) => {
    const t = e.target;
    // A field that somehow ended up inside the text is text, not one of the composer's own fields.
    if (t !== editor && editor.contains(t)) return changed();
    if (t.dataset.rinput) {
      if (/[,;]/.test(t.value)) {
        commit(t.dataset.rinput, t.value);
        t.value = '';
        renderRecipients(t.dataset.rinput).focus();
        return;
      }
      showSuggest(t);
      return;
    }
    if (t.dataset.field === 'subject') {
      st.subject = t.value;
      title();
    }
    if (t.classList.contains('color-input')) return;
    changed();
  });
  page.addEventListener('change', (e) => {
    const t = e.target;
    if (editor.contains(t)) return;
    if (t.dataset.field === 'account') {
      const [accountId, ...rest] = t.value.split('|');
      const moved = accountId !== st.accountId;
      st.accountId = accountId;
      st.from = rest.join('|');
      // Signatures belong to accounts: another address of the same account keeps the one there is.
      if (moved) {
        accountSwitches++;
        useAccountSignature();
      }
      showDefaultLink();
      changed();
    }
  });
  page.addEventListener(
    'blur',
    (e) => {
      const t = e.target;
      if (t.dataset && t.dataset.rinput && t.value.trim()) {
        commit(t.dataset.rinput, t.value);
        t.value = '';
        renderRecipients(t.dataset.rinput);
      }
      if (t.dataset && t.dataset.rinput) setTimeout(closeSuggest, 120);
    },
    true
  );

  listen(keyTarget, 'keydown', (e) => {
    if (document.querySelector('.scrim') || closing || closed) return;
    const t = e.target;
    const ctrl = e.ctrlKey || e.metaKey;
    if (ctrl && e.key === 'Enter') {
      e.preventDefault();
      return send();
    }
    if (ctrl && e.key.toLowerCase() === 's') {
      e.preventDefault();
      return saveDraft({ auto: false });
    }
    if (e.key === 'Escape') {
      if (suggest) return closeSuggest();
      if (document.querySelector('.menu')) return;
      e.preventDefault();
      e.stopPropagation();
      return close();
    }
    if (t.dataset && t.dataset.rinput) {
      if (suggest && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
        e.preventDefault();
        const buttons = [...suggest.querySelectorAll('button')];
        const i = buttons.findIndex((b) => b.classList.contains('active'));
        const next = (i + (e.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
        buttons.forEach((b, j) => b.classList.toggle('active', j === next));
        return;
      }
      if (e.key === 'Enter' || (e.key === 'Tab' && t.value.trim())) {
        if (suggest) {
          e.preventDefault();
          const i = [...suggest.querySelectorAll('button')].findIndex((b) => b.classList.contains('active'));
          return pick(suggest.hits[Math.max(0, i)], t);
        }
        if (t.value.trim()) {
          e.preventDefault();
          commit(t.dataset.rinput, t.value);
          t.value = '';
          renderRecipients(t.dataset.rinput).focus();
        }
        return;
      }
      if (e.key === 'Backspace' && !t.value && st[t.dataset.rinput].length) {
        st[t.dataset.rinput].pop();
        changed();
        renderRecipients(t.dataset.rinput).focus();
      }
    }
  });
  page.addEventListener('mousedown', (e) => {
    // Keep the editor selection when clicking formatting buttons.
    if (e.target.closest('.format-row button, .float-bar button')) e.preventDefault();
  });

  const richEditor = mountRichEditor(page, editor, { onChange: changed, onSelection: () => placeFloatBar() });
  cleanups.push(() => richEditor.dispose());
  const { editorRange, restoreRange, updateToolbar } = richEditor;

  // ----- from the calendar -----

  // At the caret when it is in the text, else above the signature (and the empty line before it), else at the end.
  function insertHtml(range, html) {
    if (range) {
      restoreRange(range);
      document.execCommand('insertHTML', false, html);
    } else {
      const tpl = document.createElement('template');
      tpl.innerHTML = html;
      editor.insertBefore(tpl.content, signatureStart() || null);
    }
    changed();
  }

  // The calendar button: the booking link while the booking page is on, and free times. The compose window's copy
  // of main's state is from when it opened, so the link is asked for again.
  async function calendarMenu(b) {
    const range = editorRange();
    const fresh = await api('state').catch(() => data);
    const url = safeBookingUrl(fresh && fresh.bookingUrl);
    showMenu(
      b,
      [
        url ? { icon: 'link', get label() { return t('composer.booking.insertLink'); }, action: () => insertHtml(range, `<a href="${esc(url)}">${esc(url)}</a>`) } : null,
        { icon: 'clock', get label() { return t('composer.booking.insertTimes'); }, action: () => freeTimes(range, fresh || data) }
      ],
      { above: true }
    );
  }

  // The free-times dialog: length, period, hours and zone, with the lines main formats (calendarFree) as a preview,
  // and what the preview shows goes into the text. With the booking link, the times follow the booking page's hours
  // and the chosen meeting type's length, notice and horizon. Buffers and daily limits only count on the page itself,
  // so the page can offer fewer times than the list.
  async function freeTimes(range, state) {
    const booking = await api('bookingState').catch(() => null);
    const url = booking && booking.status === 'on' ? safeBookingUrl(booking.url) : null;
    const types = url && booking.settings && Array.isArray(booking.settings.types) ? booking.settings.types.filter((x) => !x.hidden) : [];
    const calendar = state && state.settings && state.settings.calendar;
    const f = { length: 30, range: 'week', hours: url ? 'booking' : 'working', zone: pcZone(), link: Boolean(url), type: types[0] ? types[0].slug : null, result: null, seq: 0 };
    const MAX_LINES = 5;
    const typeOf = () => types.find((x) => x.slug === f.type) || types[0] || null;
    const linked = () => Boolean(f.link && url);
    // The footer under the signature has the link already: the times still follow the page, without a second link.
    const inFooter = () => Boolean(url) && [...editor.querySelectorAll(':scope > .signature a[href]')].some((a) => a.getAttribute('href') === url);
    const options = () => ({
      length: linked() && typeOf() ? typeOf().duration : f.length,
      hours: linked() ? 'booking' : f.hours
    });
    const html = () => {
      const r = f.result;
      if (!r || !r.lines.length) return '';
      const lines = r.lines.slice(0, MAX_LINES);
      const link = linked() && !inFooter() ? `<div>${withLink(t('composer.booking.linkLine', { url: '{url}' }), url)}</div>` : '';
      return `<div>${esc(t('composer.booking.intro'))}</div><ul>${lines.map((l) => `<li>${esc(l)}</li>`).join('')}</ul><div>${esc(t('composer.booking.zoneLine', { zone: r.timeZone }))}</div>${link}<div><br></div>`;
    };
    const picked = await dialog({
      title: t('composer.booking.timesTitle'),
      wide: true,
      buttons: [
        { get label() { return t('common.actions.cancel'); }, value: null },
        { get label() { return t('composer.booking.insert'); }, primary: true, value: () => html() || KEEP_OPEN }
      ],
      render(body) {
        const rangeLabel = { week: t('calendar.free.rangeThisWeek'), next: t('calendar.free.rangeNextWeek'), days14: t('calendar.free.range14') };
        const draw = () => {
          const lengthField =
            linked() && types.length
              ? `<label>${esc(t('composer.booking.type'))}<select name="type">${types
                  .map((x) => `<option value="${esc(x.slug)}"${x.slug === (typeOf() || {}).slug ? ' selected' : ''}>${esc(`${x.title} · ${duration(x.duration * 60000)}`)}</option>`)
                  .join('')}</select></label>`
              : `<label>${esc(t('calendar.free.length'))}<select name="length">${FREE_LENGTHS.map((n) => `<option value="${n}"${n === f.length ? ' selected' : ''}>${esc(duration(n * 60000))}</option>`).join('')}</select></label>`;
          const hours = [url ? ['booking', t('calendar.free.hoursBooking')] : null, ['working', t('calendar.free.hoursWorking')], ['any', t('calendar.free.hoursAny')]].filter(Boolean);
          const hourValue = options().hours;
          body.innerHTML = `<div class="form free-times-form" style="padding:0">
            <div class="free-times-grid">
              ${lengthField}
              <label>${esc(t('calendar.free.range'))}<select name="range">${FREE_RANGES.map((r) => `<option value="${r}"${r === f.range ? ' selected' : ''}>${esc(rangeLabel[r])}</option>`).join('')}</select></label>
              <label>${esc(t('calendar.free.hours'))}<select name="hours" ${linked() ? 'disabled' : ''}>${hours.map(([v, label]) => `<option value="${v}"${v === hourValue ? ' selected' : ''}>${esc(label)}</option>`).join('')}</select></label>
              <label>${esc(t('calendar.free.zone'))}<select name="zone">${zoneOptions(f.zone, booking && booking.settings && booking.settings.timeZone ? [booking.settings.timeZone] : [])}</select></label>
            </div>
            ${url ? `<label class="inline"><input type="checkbox" name="link" ${f.link ? 'checked' : ''}/> ${esc(t('composer.booking.withLink'))}</label><span class="field-help">${esc(t('composer.booking.withLinkHelp'))}${inFooter() ? ` ${esc(t('composer.booking.linkInFooter'))}` : ''}</span>` : ''}
            <div class="free-preview-label" id="free-preview-label">${esc(t('composer.booking.preview'))}</div>
            <div class="free-preview" aria-labelledby="free-preview-label" aria-live="polite"></div>
          </div>`;
          showPreview();
        };
        const insertButton = () => body.closest('.dialog').querySelector('.buttons button:last-child');
        const showPreview = () => {
          const box = body.querySelector('.free-preview');
          if (!box) return;
          const content = html();
          if (!f.result) box.innerHTML = `<p class="free-preview-note"><span class="spinner"></span>${esc(t('calendar.free.loading'))}</p>`;
          else if (f.result.error) box.innerHTML = `<p class="free-preview-note error">${esc(f.result.error)}</p>`;
          else if (!content) box.innerHTML = `<p class="free-preview-note">${esc(t('composer.booking.none'))}</p>`;
          else {
            // The same markup that goes into the text, parsed inert; failed accounts are said under it.
            const tpl = document.createElement('template');
            tpl.innerHTML = content;
            box.replaceChildren(tpl.content);
          }
          for (const failed of (f.result && f.result.failed) || []) box.insertAdjacentHTML('beforeend', `<p class="free-preview-note warn">${esc(t('calendar.free.failed', { email: failed.email }))}</p>`);
          const button = insertButton();
          if (button) button.disabled = !content;
        };
        const load = async () => {
          const seq = ++f.seq;
          f.result = null;
          showPreview();
          const r = freeRange(f.range, calendar);
          const o = options();
          // With the link, the type's notice and how far ahead it can be booked cut the period, as on the page. The
          // first time after the notice starts on a quarter hour.
          const type = linked() ? typeOf() : null;
          const now = Date.now();
          const start = type ? Math.max(r.start, Math.ceil((now + (Number(type.minNotice) || 0) * 60000) / 900000) * 900000) : r.start;
          const end = type ? Math.min(r.end, now + (Number(type.horizon) || 30) * 86400000) : r.end;
          let result;
          try {
            result = end > start ? await api('calendarFree', { start, end, length: o.length, hours: o.hours, timeZone: f.zone }) : { lines: [], failed: [], windows: [], timeZone: f.zone };
          } catch (err) {
            result = { error: err.message, lines: [], failed: [] };
          }
          if (seq !== f.seq || !body.isConnected) return;
          f.result = result;
          showPreview();
        };
        body.addEventListener('change', (e) => {
          const el = e.target;
          if (el.name === 'length') f.length = Number(el.value);
          if (el.name === 'type') f.type = el.value;
          if (el.name === 'range') f.range = el.value;
          if (el.name === 'hours') f.hours = el.value;
          if (el.name === 'zone') f.zone = el.value;
          if (el.name === 'link') {
            f.link = el.checked;
            draw();
            body.querySelector('[name="link"]')?.focus();
          }
          load();
        });
        draw();
        load();
        // The dialog focuses its first text field; this one starts on the first choice instead.
        setTimeout(() => body.querySelector('select')?.focus(), 0);
      }
    });
    if (picked) insertHtml(range, picked);
    else restoreRange(range);
  }

  // ----- formatting -----

  const formatRow = $('.format-row', page);
  const formatToggle = $('[data-c="format"]', page);
  const showFormatRow = (on) => {
    formatRow.hidden = !on;
    formatToggle.classList.toggle('on', on);
    formatToggle.setAttribute('aria-pressed', String(on));
    localStorage.setItem('rukoo.formatRow', on ? '1' : '');
  };
  showFormatRow(localStorage.getItem('rukoo.formatRow') === '1');

  // A small toolbar above selected text, as in most current mail apps.
  const floatBar = $('.float-bar', page);
  const placeFloatBar = () => {
    const sel = window.getSelection();
    const range = sel.rangeCount ? sel.getRangeAt(0) : null;
    // An open menu keeps it hidden: the selection a right-click makes can report after the menu has opened. A
    // selected image has a bar of its own (editor-images.js).
    if (!range || range.collapsed || !editor.contains(range.commonAncestorContainer) || closing || closed || menuOpen() || soleImage(range)) {
      floatBar.hidden = true;
      return;
    }
    const r = range.getBoundingClientRect();
    const box = page.getBoundingClientRect();
    floatBar.hidden = false;
    const w = floatBar.offsetWidth;
    const left = Math.max(8, Math.min(r.left - box.left + r.width / 2 - w / 2, box.width - w - 8));
    const top = r.top - box.top - floatBar.offsetHeight - 8;
    floatBar.style.left = `${left}px`;
    floatBar.style.top = `${top < 4 ? r.bottom - box.top + 8 : top}px`;
  };

  listen(document, 'selectionchange', () => {
    if (editor.contains(document.activeElement) || document.activeElement === editor) updateToolbar();
    placeFloatBar();
  });
  $('.composer-scroll', page).addEventListener('scroll', () => (floatBar.hidden = true));
  // Right-click in the text: spelling suggestions, cut, copy and paste. A right-click on a misspelled word selects it,
  // which brings up the formatting bar; the menu hides it again.
  cleanups.push(editMenu(editor, { onOpen: () => (floatBar.hidden = true) }));

  // ----- clicks -----

  page.addEventListener('click', async (e) => {
    const field = e.target.closest('.field[data-rfield]');
    if (field && !e.target.closest('button') && !e.target.closest('.suggest')) field.querySelector('input')?.focus();
    if (e.target.closest('.compose-body') && !e.target.closest('.editor, .quote-zone, button') && e.target === $('.compose-body', page)) editor.focus();
    const b = e.target.closest('button');
    // A button inside the text (from a pasted or agent-written body) is never one of the composer's controls.
    if (!b || editor.contains(b)) return;
    if (b.dataset.remove !== undefined) {
      const f = b.closest('[data-rfield]').dataset.rfield;
      st[f].splice(Number(b.dataset.remove), 1);
      changed();
      return renderRecipients(f).focus();
    }
    if (b.dataset.unattach !== undefined) {
      st.attachments.splice(Number(b.dataset.unattach), 1);
      changed();
      return renderAttachments();
    }
    if (b.dataset.cmd) return;
    if (b.dataset.color) {
      const input = $('.color-input', page);
      const range = editorRange();
      input.onchange = () => {
        restoreRange(range);
        document.execCommand(b.dataset.color, false, input.value);
        changed();
      };
      input.click();
      return;
    }
    switch (b.dataset.c) {
      case 'send':
        return send();
      case 'attach':
        return attachFiles();
      case 'link':
        return;
      case 'format':
        return showFormatRow(formatRow.hidden);
      case 'image': {
        const range = editorRange();
        const url = await api('pickImage');
        if (!url) return;
        restoreRange(range);
        document.execCommand('insertImage', false, url);
        changed();
        return;
      }
      case 'calendar':
        return calendarMenu(b);
      case 'cc':
      case 'bcc': {
        const key = b.dataset.c;
        st[key === 'cc' ? 'showCc' : 'showBcc'] = true;
        $(`[data-rfield="${key}"]`, page).hidden = false;
        b.hidden = true;
        return $(`[data-rinput="${key}"]`, page).focus();
      }
      case 'quote': {
        st.quoteOpen = !st.quoteOpen;
        $('.quote', page).hidden = !st.quoteOpen;
        b.setAttribute('aria-expanded', String(st.quoteOpen));
        b.title = st.quoteOpen ? t('composer.quote.hide') : t('composer.quote.show');
        if (st.quoteOpen && !quoteDrawn) drawQuote();
        return;
      }
      case 'include':
        st.include = !st.include;
        b.textContent = st.include ? t('composer.quote.exclude') : t('composer.quote.include');
        $('.quote', page).classList.toggle('excluded', !st.include);
        $('.quote-pill', page).classList.toggle('excluded', !st.include);
        $('.quote-pill', page).textContent = st.include ? '···' : t('composer.quote.excludedPill');
        changed();
        return;
      case 'discard':
        return discard();
      case 'agent-undo':
        return undoAgent();
      case 'close':
        return close();
      case 'popout':
        return popOut();
      case 'make-default':
        return makeDefault(b);
      case 'more':
        return showMenu(b, [
          { icon: 'saved', get label() { return t('composer.actions.saveDraft'); }, shortcut: 'Ctrl+S', action: () => saveDraft({ auto: false }) },
          st.showCc ? null : { icon: 'plus', get label() { return t('composer.actions.addCc'); }, action: () => $('[data-c="cc"]', page).click() },
          st.showBcc ? null : { icon: 'plus', get label() { return t('composer.actions.addBcc'); }, action: () => $('[data-c="bcc"]', page).click() },
          inline ? { icon: 'popOut', get label() { return t('composer.actions.popout'); }, action: popOut } : null,
          { separator: true },
          { icon: 'trash', get label() { return t('composer.actions.discard'); }, action: discard, danger: true }
        ], { above: true });
    }
  });

  async function attachFiles() {
    const files = await api('pickFiles');
    if (files.length) {
      st.attachments.push(...files.map(keyed));
      changed();
      renderAttachments();
    }
  }

  // Files dropped on the editor become attachments.
  const dropTarget = inline ? page : document;
  listen(dropTarget, 'dragover', (e) => {
    if ([...e.dataTransfer.types].includes('Files')) {
      e.preventDefault();
      page.classList.add('drop-files');
    }
  });
  listen(dropTarget, 'dragleave', (e) => {
    if (!e.relatedTarget || !page.contains(e.relatedTarget)) page.classList.remove('drop-files');
  });
  listen(dropTarget, 'drop', (e) => {
    page.classList.remove('drop-files');
    const files = [...(e.dataTransfer.files || [])];
    if (!files.length) return;
    e.preventDefault();
    for (const f of files) {
      const p = window.mail.pathForFile(f);
      if (p) st.attachments.push(keyed({ path: p, filename: f.name, size: f.size }));
    }
    changed();
    renderAttachments();
  });

  // ----- saving, sending, closing -----

  // sending: the payload for Send rather than a draft, which links mentions only to people in To and Cc.
  function payload(attachmentList = st.attachments, { sending = false } = {}) {
    let html = outgoingHtml(editor, sending ? new Set([...st.to, ...st.cc].map((a) => String(a.address || '').toLowerCase())) : null);
    if ((st.original || st.quoted) && st.include) {
      const head = esc(st.original ? quoteHeader(st.original) : st.quoted.head).replace(/\n/g, '<br>');
      const quoted = st.original ? st.original.html || '' : st.quoted.html;
      html += `<br><div data-rukoo-quote style="font-size:13px;color:#555">${head}</div><br><blockquote style="margin:0 0 0 .8ex;border-left:1px solid #ccc;padding-left:1ex">${quoted}</blockquote>`;
    }
    const attachments = attachmentList.filter((a) => a.path).map((a) => ({ path: a.path, filename: a.filename }));
    const forwardIndexes = attachmentList.filter((a) => a.forwardIndex !== undefined).map((a) => a.forwardIndex);
    return {
      accountId: st.accountId,
      from: st.from,
      to: st.to,
      cc: st.cc,
      bcc: st.bcc,
      subject: st.subject,
      html: `<div style="font-family:'Segoe UI',Arial,sans-serif;font-size:12pt">${html}</div>`,
      attachments,
      forwardId: forwardIndexes.length ? st.forwardId : null,
      forwardIndexes,
      inReplyTo: st.inReplyTo,
      references: st.references,
      draftId: st.draftId,
      replyToId: st.replyToId
    };
  }

  function flushInputs() {
    page.querySelectorAll('[data-rinput]').forEach((input) => {
      if (input.value.trim()) {
        commit(input.dataset.rinput, input.value);
        input.value = '';
        renderRecipients(input.dataset.rinput);
      }
    });
  }

  // A save replaces the stored draft, and with it the source of the attachments copied from it.
  // The new draft holds what the save sent: files from disk first, then copied ones. Point those
  // entries at the new draft; attachments added or removed during the save stay as they are.
  function remapAttachments(sent, saved) {
    const order = [...sent.filter((a) => a.path), ...sent.filter((a) => a.forwardIndex !== undefined)];
    const stored = [...(saved.attachments || [])].sort((a, b) => a.index - b.index);
    const pairs = new Map();
    if (stored.length === order.length) {
      order.forEach((a, i) => pairs.set(a.key, stored[i]));
    } else {
      const free = [...stored];
      for (const a of order) {
        const i = free.findIndex((x) => x.filename === a.filename);
        if (i < 0) return false;
        pairs.set(a.key, free.splice(i, 1)[0]);
      }
    }
    st.attachments = st.attachments.map((a) =>
      pairs.has(a.key) ? { key: a.key, forwardIndex: pairs.get(a.key).index, filename: pairs.get(a.key).filename, size: pairs.get(a.key).size } : a
    );
    return true;
  }

  // Set after a save whose attachments could not be read back yet; settled before the next save or send.
  let pendingRemap = null;
  async function settleAttachments() {
    if (!pendingRemap) return;
    const { id, sent } = pendingRemap;
    const lost = new Error(t('composer.errors.missingAttachments'));
    if (!id) throw lost;
    const saved = await api('get', id);
    if (!remapAttachments(sent, saved)) throw lost;
    st.forwardId = id;
    pendingRemap = null;
    renderAttachments();
  }

  // Saves run one at a time; a save requested during another runs after it.
  let saving = Promise.resolve(true);
  function saveDraft({ auto }) {
    clearTimeout(autosaveTimer);
    saving = saving.then(async () => {
      if (sending || closed) return false;
      if (auto && !st.dirty) return true;
      flushInputs();
      st.saving = true;
      describeSaved();
      const editsBefore = editCount;
      let failed = false;
      try {
        await settleAttachments();
        const sent = st.attachments.slice();
        const id = await api('saveDraft', payload(sent));
        st.draftId = id;
        st.savedAt = Date.now();
        if (editCount === editsBefore) st.dirty = false;
        if (id && !inline) await api('composeDraft', id);
        // Without an id the next save cannot replace this one; stop autosaving rather than pile up drafts.
        if (!id) autosave = false;
        if (sent.length) {
          pendingRemap = { id, sent };
          await settleAttachments().catch(() => {});
        }
        if (!auto) toast(t('composer.status.savedToDrafts'));
        return true;
      } catch (err) {
        failed = true;
        if (!auto) toast(err.message, 5000);
        return false;
      } finally {
        st.saving = false;
        if (failed && auto) showSaveState(t('composer.errors.autosave'));
        else describeSaved();
      }
    });
    return saving;
  }

  let closed = false;
  let closing = false;
  function finish(message) {
    closed = true;
    clearTimeout(autosaveTimer);
    cleanups.forEach((fn) => fn());
    if (inline) {
      if (message) toast(message);
      page.remove();
      onDone && onDone();
      return;
    }
    if (message) api('toastMain', message).catch(() => {});
    api('composeClose');
  }

  let sending = false;
  // While sending: resolves to true once the message is sent and the editor closed, false when it stays open.
  let sent = null;
  function send() {
    // Taken before anything is awaited, so a second click cannot send the message twice.
    // Nothing is sent once the editor is saving to close.
    if (sending || closing || closed) return;
    sending = true;
    const outcome = () => closed;
    sent = trySend()
      .then(outcome, outcome)
      .finally(() => (sent = null));
    return sent;
  }

  async function trySend() {
    const btn = page.querySelector('[data-c="send"]');
    const label = btn.querySelector('span');
    btn.disabled = true;
    let done = false;
    try {
      flushInputs();
      if (!st.to.length && !st.cc.length && !st.bcc.length) {
        toast(t('composer.errors.noRecipients'));
        return $('[data-rinput="to"]', page).focus();
      }
      const bad = [...st.to, ...st.cc, ...st.bcc].find((a) => !EMAIL_RE.test(a.address));
      if (bad) return toast(t('common.errors.invalidEmailValue', { address: bad.address }));
      if (!st.subject.trim()) {
        const ok = await confirmDialog(t('composer.send.noSubjectTitle'), t('composer.send.noSubjectHelp'), t('composer.actions.send'));
        if (!ok) return;
      }
      clearTimeout(autosaveTimer);
      label.textContent = t('composer.status.sending');
      showSaveState('');
      // Nothing typed from here on would make it into the mail, so lock the editor until it is sent.
      page.inert = true;
      await saving;
      await settleAttachments();
      await api('send', payload(undefined, { sending: true }));
      done = true;
      gone = true;
      finish(inline ? t('composer.status.sent') : null);
    } catch (err) {
      await dialog({ title: t('composer.errors.send'), body: `<p>${esc(err.message)}</p>` });
    } finally {
      if (!done) {
        sending = false;
        page.inert = false;
        btn.disabled = false;
        label.textContent = t('composer.actions.send');
        describeSaved();
        if (st.dirty) armAutosave();
      }
    }
  }

  async function discard() {
    if (sending || closing || closed) return;
    const hasContent = st.dirty || st.draftId;
    if (hasContent) {
      const ok = await confirmDialog(t('composer.discard.title'), st.draftId ? t('composer.discard.savedHelp') : t('composer.discard.unsavedHelp'), t('common.actions.delete'), true);
      if (!ok) return;
    }
    clearTimeout(autosaveTimer);
    closed = true;
    gone = true;
    await saving;
    if (st.draftId) await api('discardDraft', st.draftId).catch(() => {});
    finish(st.draftId ? t('composer.status.deleted') : null);
  }

  let asking = false;
  let answer = null;
  // Resolves to true once the editor has closed, false while it stays open (Cancel, or a save that failed). Called
  // again while the question or its save is still open, it returns the same answer: a quit waits for it.
  function close() {
    if (closed) return Promise.resolve(true);
    if (answer) return answer;
    // A quit during a send goes on once the message is sent, and is called off if it isn't.
    if (sent) return sent;
    // Busy with another question (the agent's Undo) or a save before closing.
    if (asking || closing || sending) return Promise.resolve(false);
    answer = ask().finally(() => (answer = null));
    return answer;
  }

  async function ask() {
    flushInputs();
    if (!st.dirty) {
      finish(st.draftId && st.mode !== 'draft' ? t('composer.status.kept') : null);
      return true;
    }
    asking = true;
    const choice = await dialog({
      title: t('composer.close.title'),
      body: `<p data-i18n="composer.close.help">${esc(t('composer.close.help'))}</p>`,
      buttons: [
        { label: st.draftId ? t('composer.actions.discard') : t('composer.close.discard'), value: 'discard', danger: true },
        { get label() { return t('common.actions.cancel'); }, value: null },
        { get label() { return t('common.actions.save'); }, value: 'save', primary: true }
      ]
    });
    asking = false;
    if (choice === 'save') return saveAndFinish(t('composer.status.savedToDrafts'));
    if (choice === 'discard') {
      closed = true;
      gone = true;
      await saving;
      if (st.draftId) await api('discardDraft', st.draftId).catch(() => {});
      finish(st.draftId ? t('composer.status.deleted') : null);
      return true;
    }
    return false;
  }

  // Lock the editor while the last save runs, so nothing typed now is left out of it.
  async function saveAndFinish(message) {
    closing = true;
    page.inert = true;
    const ok = await saveDraft({ auto: false });
    if (ok && !st.dirty) {
      finish(message);
      return true;
    }
    closing = false;
    page.inert = false;
    return false;
  }

  // Moving on to another message: keep what was written as a draft without asking.
  async function leave() {
    if (closed) return true;
    if (sending || closing || asking) return false;
    flushInputs();
    if (!st.dirty) {
      finish(st.draftId && st.mode !== 'draft' ? t('composer.status.kept') : null);
      return true;
    }
    return saveAndFinish(t('composer.status.savedClosed'));
  }

  async function popOut() {
    if (sending || closing || closed) return;
    flushInputs();
    if (st.dirty || st.draftId) {
      if (st.dirty && !(await saveAndFinish(null))) return;
      if (!closed) finish(null);
      if (st.draftId) onPopOut && onPopOut({ mode: 'draft', id: st.draftId });
      return;
    }
    finish(null);
    onPopOut && onPopOut(opts);
  }

  // ----- agent writes -----

  const showField = (field, on) => {
    st[field === 'cc' ? 'showCc' : 'showBcc'] = on;
    $(`[data-rfield="${field}"]`, page).hidden = !on;
    $(`[data-c="${field}"]`, page).hidden = on;
  };
  const setSubject = (subject) => {
    st.subject = subject;
    $('[data-field="subject"]', page).value = subject;
    title();
  };
  const snapshot = () => ({ html: editor.innerHTML, accountSwitches, to: [...st.to], cc: [...st.cc], bcc: [...st.bcc], subject: st.subject, showCc: st.showCc, showBcc: st.showBcc });
  // Agent text goes above the signature and the empty line before it.
  const signatureStart = () => {
    const sig = editor.querySelector(':scope > .signature');
    const spacer = sig && sig.previousElementSibling;
    return spacer && !spacer.textContent.trim() && !spacer.querySelector('img') ? spacer : sig;
  };
  // The chosen account's signature and booking link in place of the ones in the editor, as a new message from that
  // account would have them: added at the end when there were none, gone with their empty line when it has none.
  const useAccountSignature = () => {
    const acc = data.accounts.find((a) => a.id === st.accountId);
    if (!acc) return;
    const html = signatureBlockHtml(acc, data);
    const sig = editor.querySelector(':scope > .signature');
    if (!html) {
      if (sig) signatureStart().remove();
      sig?.remove();
      return;
    }
    // Parsed as an inert template, then moved in.
    const tpl = document.createElement('template');
    tpl.innerHTML = html;
    if (sig) return sig.replaceChildren(tpl.content);
    const spacer = document.createElement('div');
    spacer.append(document.createElement('br'));
    const block = document.createElement('div');
    block.className = 'signature';
    block.append(tpl.content);
    editor.append(spacer, block);
  };

  // content: { to, cc, bcc, subject, html | text }. A field left out stays as it is; an empty list clears it.
  // writeId: the chat panel's id for this write, so its draft card can tell whether Undo still takes it back.
  // Never sends: the user reviews and sends. The quote stays outside the editor and is added on save.
  function setContent(content = {}, { mode = 'replace', by = null, writeId = null } = {}) {
    // Technical detail for the agent, so not translated.
    if (sending || closing || closed || asking || page.inert) throw Object.assign(new Error('The composer is busy sending, saving or closing.'), { code: 'busy' });
    // Several writes in a row keep the snapshot from before the first; after your own edit a new one is taken.
    if (!agentBefore || !st.agent || st.agent.edited) {
      agentBefore = snapshot();
      covered = new Set();
    }
    if (writeId) covered.add(String(writeId));
    for (const field of ['to', 'cc', 'bcc']) {
      if (!Array.isArray(content[field])) continue;
      const list = addressList(content[field]);
      st[field] = list;
      if (field !== 'to') showField(field, list.length > 0);
      renderRecipients(field);
    }
    if (content.subject != null) setSubject(String(content.subject).slice(0, 1000));
    if (content.html != null || content.text != null) {
      const clean = content.html != null ? sanitizeAgentHtml(content.html) : textToEditorHtml(content.text);
      const stop = signatureStart();
      if (mode === 'replace') while (editor.firstChild && editor.firstChild !== stop) editor.firstChild.remove();
      // Parsed as an inert template, then moved in.
      const tpl = document.createElement('template');
      tpl.innerHTML = clean || '<div><br></div>';
      editor.insertBefore(tpl.content, stop || null);
    }
    changed({ by: by || {} });
    return getDraft();
  }

  // What is in the composer now, for the agent and the chat panel. Reads only; changes nothing.
  function getDraft() {
    const typed = (field) => ($(`[data-rinput="${field}"]`, page)?.value || '').trim();
    const people = (list) => list.map((a) => ({ name: a.name || '', address: a.address }));
    return {
      key,
      mode: st.mode,
      messageRef: st.mode !== 'draft' && opts.id ? opts.id : null,
      accountId: st.accountId,
      from: st.from,
      to: people(st.to),
      cc: people(st.cc),
      bcc: people(st.bcc),
      subject: st.subject,
      html: editor.innerHTML,
      text: editor.innerText,
      pendingRecipients: { to: typed('to'), cc: typed('cc'), bcc: typed('bcc') },
      quote: st.original
        ? { from: st.original.from || null, date: st.original.date || null, subject: st.original.subject || '' }
        : st.quoted
          ? { head: st.quoted.head }
          : null,
      includeQuote: st.include,
      attachments: st.attachments.map(({ filename, size }) => ({ filename, size: size || 0 })),
      draftId: st.draftId,
      replyToId: st.replyToId,
      inReplyTo: st.inReplyTo,
      dirty: st.dirty,
      // Any edit of the user's own since this composer opened, saved or not; and a counter that moves on
      // every change, so a caller that waited can tell whether the draft changed under it.
      userEdited: Boolean(st.userEdits),
      revision: editCount,
      saving: Boolean(st.saving),
      sending,
      savedAt: st.savedAt,
      agent: st.agent ? { name: st.agent.name, conversationId: st.agent.conversationId, edited: st.agent.edited } : null
    };
  }

  // Whether Undo still takes back this write (or, without an id, any agent write).
  const canUndoAgent = (writeId) => Boolean(agentBefore) && (writeId == null || covered.has(String(writeId)));

  // Puts back what was there before the agent wrote. After your own edits it asks first.
  // writeId: only when that write is what Undo takes back now.
  async function undoAgent(writeId) {
    if (!canUndoAgent(writeId) || sending || closing || closed || asking) return false;
    const target = agentBefore;
    if (st.agent && st.agent.edited) {
      // Agent writes wait while the question is open, so the answer applies to the snapshot it was asked about.
      asking = true;
      let ok = false;
      try {
        ok = await confirmDialog(t('composer.agent.undoTitle', { name: st.agent.name }), t('composer.agent.undoHelp'), t('composer.agent.undo'));
      } finally {
        asking = false;
      }
      if (!ok || agentBefore !== target || sending || closing || closed) return false;
    }
    const before = agentBefore;
    const writeIds = [...covered];
    agentBefore = null;
    covered = new Set();
    st.agent = null;
    editor.innerHTML = before.html;
    // From stays as you set it, so after a change of account (also away and back) the signature is that account's now.
    if (before.accountSwitches !== accountSwitches) useAccountSignature();
    for (const field of ['to', 'cc', 'bcc']) {
      st[field] = before[field];
      renderRecipients(field);
    }
    showField('cc', before.showCc);
    showField('bcc', before.showBcc);
    setSubject(before.subject);
    changed();
    // The chat panel marks the draft cards of these writes as undone.
    page.dispatchEvent(new CustomEvent('agent-undo', { bubbles: true, detail: { key, writeIds } }));
    return true;
  }

  // ----- @-mentions -----

  mountMentions({
    editor,
    st,
    getData: () => data,
    listen,
    onClose: (fn) => cleanups.push(fn),
    changed,
    addCc: (person) => {
      st.cc.push(person);
      showField('cc', true);
      renderRecipients('cc');
    }
  });

  if (!inline) {
    // The window's close button: keep the window open while there are unsaved changes and ask instead.
    listen(window, 'beforeunload', (e) => {
      if (closed) return;
      if (!st.dirty && !sending && !closing) {
        if (st.draftId && st.mode !== 'draft') api('toastMain', t('composer.status.kept')).catch(() => {});
        return;
      }
      e.preventDefault();
      e.returnValue = false;
      // While sending or saving before closing, the window closes by itself when that is done; otherwise this
      // asks. Main hears the outcome, because a quit waits for it.
      setTimeout(() => close().then((done) => api('unloadAnswer', done)).catch(() => {}), 0);
    });
  }

  const focusStart = () => {
    if (st.mode === 'forward' || !st.to.length) return $('[data-rinput="to"]', page).focus();
    editor.focus();
    const range = document.createRange();
    range.setStart(editor.firstChild || editor, 0);
    range.collapse(true);
    window.getSelection().removeAllRanges();
    window.getSelection().addRange(range);
  };
  // A composer an agent opens while you type in the chat leaves the focus where it is.
  if (opts.focus !== false) focusStart();
  // Seeded by an agent: dirty from the start, so closing or moving on keeps it as a draft.
  if (opts.agent) changed({ by: opts.agent });
  // Filled in by a mailto: link (or an unsubscribe by email): there is something to lose before anyone types, so
  // closing asks and autosave keeps it. Not through changed(), which would count the link's words as the user's own
  // edits. Recipients alone don't count, so a new message to a contact leaves no empty draft behind. A reply or
  // forward nobody touched still closes without asking: it holds nothing the email it answers doesn't.
  else if (prefilled(st, opts)) {
    st.dirty = true;
    describeSaved();
    armAutosave();
  }

  return {
    el: page,
    opts,
    get draftId() {
      return st.draftId;
    },
    isDirty: () => st.dirty,
    isBusy: () => sending || closing || asking,
    key,
    // The stored draft this composer leaves behind, or null once it was sent or deleted.
    get savedDraftId() {
      return gone ? null : st.draftId;
    },
    leave,
    close,
    focus: focusStart,
    setContent,
    getDraft,
    undoAgent,
    canUndoAgent,
    // Main's state after an update: the link beside From follows the default account, and a later change of From
    // brings the signature as Settings has it now.
    accountsChanged(next) {
      data = next;
      syncDefault(next.accounts);
    },
    retheme(next) {
      if (next) {
        data = next;
        syncDefault(next.accounts);
      }
      localize(page);
      page.setAttribute('aria-label', COMPOSE_TITLES[st.mode] || t('composer.titles.new'));
      const heading = page.querySelector('.composer-title');
      if (heading) heading.textContent = COMPOSE_TITLES[st.mode] || t('composer.titles.new');
      title();
      describeSaved();
      describeAgent();
      renderAttachments();
      page.querySelector('[data-c="quote"]')?.setAttribute('title', t(st.quoteOpen ? 'composer.quote.hide' : 'composer.quote.show'));
      const include = page.querySelector('[data-c="include"]');
      if (include) include.textContent = t(st.include ? 'composer.quote.exclude' : 'composer.quote.include');
      const pill = page.querySelector('.quote-pill');
      if (pill) pill.textContent = st.include ? '···' : t('composer.quote.excludedPill');
      if (st.original) page.querySelector('.quote-head').textContent = quoteHeader(st.original);
      if (quoteDrawn) drawQuote();
    }
  };
}
