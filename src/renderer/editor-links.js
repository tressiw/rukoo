// Links in the editor that mail and meeting notes share (rich-editor.js): addresses become links as you paste or
// type them, a pasted address links the selected text, Ctrl+K opens a popover with the text and the address, and a
// link under the caret gets a bubble to open, change or remove it.
//
// Every change goes through the editing commands, so Ctrl+Z takes it back in one step.
import { t } from './i18n.js';
import { icons } from './icons.js';
import { api, esc, toast } from './ui.js';
import { soleUrl, typedUrl, linkedHtml, linkHref, shortUrlParts, bareHref } from './linkify.js';

// Text in here is never linked: it is a link already, or code.
const NO_LINK = 'a, code, pre';
// The addresses main opens (openUrl in main.js).
const OPENS = /^(https?:|mailto:)/i;
const GAP = 6;
const EDGE = 8;

// Puts a bubble or popover el under its anchor at, or above it where the window has no room below; never past the
// window's edge. The link's bubble, its popover and the image bar (editor-images.js) share it. el is measured at the
// window's left edge: a box whose buttons wrap is as wide as the room right of it, so the size it has there is the
// size it keeps where it goes. Measured where it was shown last, it could come out narrower and taller than it is
// once it moves, and end at the window's edge.
export function placePopover(el, at) {
  el.style.left = '0px';
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  const below = at.bottom + GAP;
  const fits = below + h <= window.innerHeight - EDGE;
  const top = fits || at.top - GAP - h < EDGE ? below : at.top - GAP - h;
  el.style.left = `${Math.max(EDGE, Math.min(at.left, window.innerWidth - w - EDGE))}px`;
  el.style.top = `${Math.max(EDGE, Math.min(top, window.innerHeight - h - EDGE))}px`;
}

// editor: the contenteditable element. editorRange() and restoreRange(range): the editor's selection, from
// rich-editor.js. onChange(): the editor's own bookkeeping after an edit.
// Returns { insertLink(), dispose() }; insertLink() opens the popover, as Ctrl+K and the toolbar's link button do.
export function mountEditorLinks(editor, { editorRange, restoreRange, onChange }) {
  const cleanups = [];
  const listen = (el, type, fn, opt) => {
    el.addEventListener(type, fn, opt);
    cleanups.push(() => el.removeEventListener(type, fn, opt));
  };

  // Both sit in the top layer, so no pane clips them, and right after the editor, so Tab reaches them from the text.
  const pop = document.createElement('div');
  pop.className = 'link-pop';
  pop.popover = 'manual';
  pop.setAttribute('role', 'dialog');
  pop.innerHTML = `
    <label><span data-i18n="composer.link.text">${esc(t('composer.link.text'))}</span><input type="text" data-f="text" autocomplete="off" spellcheck="false"/></label>
    <label><span data-i18n="composer.link.url">${esc(t('composer.link.url'))}</span><input type="text" data-f="url" placeholder="https://" autocomplete="off" spellcheck="false"/></label>
    <button class="btn primary sm" type="button" data-a="apply" data-i18n="composer.link.apply">${esc(t('composer.link.apply'))}</button>`;
  const bubble = document.createElement('div');
  bubble.className = 'link-bubble';
  bubble.popover = 'manual';
  bubble.setAttribute('role', 'toolbar');
  bubble.setAttribute('data-i18n-aria-label', 'composer.link.options');
  bubble.setAttribute('aria-label', t('composer.link.options'));
  bubble.innerHTML = `
    <button class="link-go" type="button" data-a="go">${icons.popOut}<span class="link-go-url"><span class="link-go-site"><bdi></bdi></span><span class="link-go-path"></span></span></button>
    <span class="sep"></span>
    <button type="button" data-a="change" data-i18n="composer.link.change">${esc(t('composer.link.change'))}</button>
    <button type="button" data-a="remove" data-i18n="composer.link.remove">${esc(t('composer.link.remove'))}</button>`;
  editor.after(bubble, pop);
  cleanups.push(() => {
    bubble.remove();
    pop.remove();
  });
  const textInput = pop.querySelector('[data-f="text"]');
  const urlInput = pop.querySelector('[data-f="url"]');
  const applyBtn = pop.querySelector('[data-a="apply"]');

  const select = (range) => {
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  };
  const selectContents = (node) => {
    const range = document.createRange();
    range.selectNodeContents(node);
    select(range);
  };
  const within = (node, selector) => {
    const el = node && (node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement);
    const found = el && el.closest(selector);
    return found && found !== editor && editor.contains(found) ? found : null;
  };
  // The link node is in. A mention is a link too, but a locked one with a menu of its own (mentions.js).
  const linkOf = (node) => {
    const a = within(node, 'a[href]');
    return a && a.isContentEditable && !a.classList.contains('mention') ? a : null;
  };
  // The link that holds the whole range, a caret included.
  const linkAt = (range) => {
    const a = range ? linkOf(range.startContainer) : null;
    return a && a.contains(range.endContainer) ? a : null;
  };
  const focusInEditor = () => document.activeElement === editor || editor.contains(document.activeElement);

  // ----- placing -----

  // Where the caret or the selection is on screen. A caret on an empty line has no box of its own; its line has.
  const rangeRect = (range) => {
    if (range && editor.contains(range.commonAncestorContainer)) {
      const rects = [...range.getClientRects()].filter((r) => r.width || r.height);
      if (rects.length) return range.collapsed ? rects[0] : range.getBoundingClientRect();
      const at = range.startContainer;
      const el = at.nodeType === Node.ELEMENT_NODE ? at.childNodes[range.startOffset] || at : at.parentElement;
      const r = (el.nodeType === Node.ELEMENT_NODE ? el : at).getBoundingClientRect();
      return { left: r.left, top: r.top, bottom: Math.min(r.bottom, r.top + 24) };
    }
    const r = editor.getBoundingClientRect();
    return { left: r.left, top: r.top, bottom: r.top + 24 };
  };
  // A link that wraps has a box for each line: the one the caret is on.
  const linkRect = (link) => {
    const rects = [...link.getClientRects()].filter((r) => r.width || r.height);
    if (!rects.length) return link.getBoundingClientRect();
    const range = editorRange();
    const caret = range && link.contains(range.startContainer) ? rangeRect(range) : null;
    const mid = caret ? (caret.top + caret.bottom) / 2 : null;
    return (caret && rects.find((r) => mid >= r.top && mid <= r.bottom)) || rects[0];
  };

  // ----- the bubble -----

  // shown: the link the bubble is open for. quiet: the link whose bubble Escape or an edit closed; it stays closed
  // until the caret leaves that link or the link is clicked.
  let shown = null;
  let quiet = null;
  const hideBubble = () => {
    shown = null;
    if (bubble.matches(':popover-open')) bubble.hidePopover();
  };
  const showBubble = (link) => {
    const href = bareHref(link.getAttribute('href'));
    const go = bubble.querySelector('[data-a="go"]');
    const [site, path] = shortUrlParts(href);
    go.querySelector('.link-go-site bdi').textContent = site;
    go.querySelector('.link-go-path').textContent = path;
    go.title = t('composer.link.open', { url: href });
    go.setAttribute('aria-label', go.title);
    shown = link;
    if (!bubble.matches(':popover-open')) bubble.showPopover();
    placePopover(bubble, linkRect(link));
  };
  // After an edit the interface rests: the link at the caret keeps its bubble closed.
  const hush = () => {
    quiet = linkAt(editorRange());
    hideBubble();
  };
  const syncBubble = () => {
    if (state) return;
    const range = editorRange();
    if (!range) {
      if (!bubble.contains(document.activeElement)) hideBubble();
      return;
    }
    // A selection gets the formatting bar; the bubble is for a caret.
    const link = range.collapsed ? linkAt(range) : null;
    if (!link) {
      if (!linkAt(range)) quiet = null;
      return hideBubble();
    }
    if (link === quiet) return hideBubble();
    quiet = null;
    if (focusInEditor()) showBubble(link);
  };

  function removeLink(link) {
    const sel = window.getSelection();
    const caret = sel.rangeCount && link.contains(sel.anchorNode) ? { node: sel.anchorNode, offset: sel.anchorOffset } : null;
    hideBubble();
    editor.focus();
    selectContents(link);
    document.execCommand('unlink');
    // The text stays where it was, so the caret can too.
    if (caret && caret.node.isConnected) window.getSelection().collapse(caret.node, Math.min(caret.offset, caret.node.nodeType === Node.TEXT_NODE ? caret.node.length : caret.node.childNodes.length));
    else window.getSelection().collapseToEnd();
    onChange();
  }

  listen(document, 'selectionchange', syncBubble);
  // A click in a link never follows it; it opens the bubble, also after Escape closed it.
  listen(editor, 'click', (e) => {
    const link = linkOf(e.target);
    if (!link) return;
    e.preventDefault();
    if (state || !window.getSelection().isCollapsed) return;
    quiet = null;
    showBubble(link);
  });
  listen(editor, 'auxclick', (e) => {
    if (linkOf(e.target)) e.preventDefault();
  });
  listen(editor, 'focusout', (e) => {
    if (!bubble.contains(e.relatedTarget)) hideBubble();
  });
  listen(bubble, 'focusout', (e) => {
    if (e.relatedTarget !== editor && !bubble.contains(e.relatedTarget)) hideBubble();
  });
  // The editor keeps the focus and the caret when the bubble is clicked.
  listen(bubble, 'mousedown', (e) => e.preventDefault());
  listen(bubble, 'click', (e) => {
    const b = e.target.closest('button');
    const link = shown;
    if (!b || !link) return;
    if (!link.isConnected) return hideBubble();
    if (b.dataset.a === 'go') {
      const href = bareHref(link.getAttribute('href'));
      // Main ignores any other address, such as a javascript: or file: link from pasted HTML; the click says so.
      if (OPENS.test(href)) api('openExternal', href).catch(() => {});
      else toast(t('composer.link.cantOpen'));
      editor.focus();
    } else if (b.dataset.a === 'change') {
      editor.focus();
      insertLink(link);
    } else if (b.dataset.a === 'remove') removeLink(link);
  });
  listen(bubble, 'keydown', (e) => {
    const buttons = [...bubble.querySelectorAll('button')];
    const i = buttons.indexOf(document.activeElement);
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      quiet = shown;
      hideBubble();
      editor.focus();
    } else if ((e.key === 'ArrowRight' || e.key === 'ArrowLeft') && i >= 0) {
      e.preventDefault();
      buttons[(i + (e.key === 'ArrowRight' ? 1 : -1) + buttons.length) % buttons.length].focus();
    }
  });
  listen(editor, 'keydown', (e) => {
    if (e.key !== 'Escape' || !shown) return;
    // Only the bubble closes, not the composer.
    e.preventDefault();
    e.stopPropagation();
    quiet = shown;
    hideBubble();
  });
  listen(document, 'scroll', (e) => {
    if (pop.contains(e.target)) return;
    hideBubble();
    if (state) placePopover(pop, state.link && state.link.isConnected ? linkRect(state.link) : rangeRect(state.range));
  }, true);
  listen(window, 'resize', () => {
    hideBubble();
    if (state) closePop({ restore: true });
  });

  // ----- the popover -----

  // state: { range, link, text, shows } while the popover is open: the selection it opened on, the link it changes,
  // the text its Text field started with and whether the link or the selection shows anything, text or an image.
  let state = null;
  const updateApply = () => {
    applyBtn.disabled = !urlInput.value.trim();
  };

  function closePop({ restore }) {
    if (!state) return;
    const { range } = state;
    state = null;
    if (pop.matches(':popover-open')) pop.hidePopover();
    if (restore && editor.isConnected) restoreRange(range && editor.contains(range.commonAncestorContainer) ? range : null);
  }

  // Opens the popover under the selection, or for link: the link at the caret when there is one.
  function insertLink(link = null) {
    if (state) return urlInput.focus();
    const range = editorRange();
    const target = link || linkAt(range);
    hideBubble();
    const text = target ? target.textContent : range ? range.toString() : '';
    textInput.value = text;
    const content = target || (range && !range.collapsed ? range.cloneContents() : null);
    // The field drops line breaks, so the text to compare with later is the one it shows.
    state = { range, link: target, text: textInput.value, shows: Boolean(content && (content.textContent.trim() || content.querySelector('img'))) };
    urlInput.value = target ? target.getAttribute('href') || '' : soleUrl(text) ? text.trim() : '';
    pop.setAttribute('aria-label', t(target ? 'composer.link.edit' : 'composer.link.title'));
    updateApply();
    pop.showPopover();
    placePopover(pop, target ? linkRect(target) : rangeRect(range));
    urlInput.focus();
    urlInput.select();
  }

  function apply() {
    const url = urlInput.value.trim();
    if (!state || !url) return;
    const href = linkHref(url);
    const typed = textInput.value;
    const { range, text, shows } = state;
    const link = state.link && state.link.isConnected ? state.link : null;
    closePop({ restore: false });
    // The text was drawn again while the popover was open (meeting notes that arrived): nothing to link any more.
    if (range && !editor.contains(range.commonAncestorContainer)) return editor.focus();
    restoreRange(range);
    // An untouched Text field keeps what the link or the selection holds, its formatting and images too, and only
    // changes the address. With nothing to show, such as an empty Text field at a caret, the address is the text.
    const keep = typed === text && shows;
    if (link) {
      if (keep) selectContents(link);
      else {
        const around = document.createRange();
        around.selectNode(link);
        select(around);
      }
    }
    if (keep) document.execCommand('createLink', false, href);
    else document.execCommand('insertHTML', false, `<a href="${esc(href)}">${esc(typed.trim() ? typed : url)}</a>`);
    window.getSelection().collapseToEnd();
    hush();
    onChange();
  }

  listen(pop, 'input', updateApply);
  listen(pop, 'click', (e) => {
    if (e.target.closest('[data-a="apply"]')) apply();
  });
  listen(pop, 'keydown', (e) => {
    // Keys in the popover stay there: Ctrl+Enter must not send the email from the Link field.
    e.stopPropagation();
    if (e.key === 'Escape') {
      e.preventDefault();
      closePop({ restore: true });
    } else if (e.key === 'Enter' && e.target.matches('input')) {
      e.preventDefault();
      apply();
    } else if (e.key === 'Tab') {
      // Tab stays inside while it is open.
      const items = [textInput, urlInput, applyBtn].filter((x) => !x.disabled);
      const first = items[0];
      const last = items[items.length - 1];
      const wrap = e.shiftKey ? document.activeElement === first && last : document.activeElement === last && first;
      if (!wrap) return;
      e.preventDefault();
      wrap.focus();
      // As Tab does on its own: a field's text is selected, so typing replaces it.
      if (wrap.select) wrap.select();
    }
  });
  // A click anywhere else closes it, and the click does what it came for: in the text it moves the caret, elsewhere
  // the selection comes back first. closedBy: the link button whose press closed it, so its click doesn't open it again.
  let closedBy = null;
  listen(document, 'mousedown', (e) => {
    closedBy = null;
    if (!state || pop.contains(e.target)) return;
    closedBy = e.target.closest ? e.target.closest('[data-c="link"]') : null;
    closePop({ restore: !editor.contains(e.target) });
  }, true);

  // ----- addresses become links -----

  const blocked = (range) => Boolean(within(range.startContainer, NO_LINK) || within(range.endContainer, NO_LINK));
  // What a selection holds in between, which createLink would wrap too.
  const holds = (range, selector) => Boolean(range.cloneContents().querySelector(selector));

  listen(editor, 'paste', (e) => {
    const data = e.clipboardData;
    const range = editorRange();
    const text = data ? data.getData('text/plain') : '';
    if (!range || !text) return;
    // Only an address on the clipboard and text selected: the address links that text instead of replacing it,
    // and a selected link gets it as its new address.
    const href = range.collapsed ? null : soleUrl(text);
    if (href) {
      const link = linkAt(range);
      if (!link && (blocked(range) || holds(range, 'code, pre, a.mention'))) return;
      e.preventDefault();
      if (link) selectContents(link);
      document.execCommand('createLink', false, href);
      window.getSelection().collapseToEnd();
      hush();
      return;
    }
    // Pasted HTML brings its own links, and Chromium cleans it; only plain text is read here.
    if (data.types.includes('text/html') || data.files.length || blocked(range)) return;
    const list = within(range.startContainer, 'li');
    const html = linkedHtml(text, list ? 'li' : 'div');
    if (!html) return;
    e.preventDefault();
    const tag = list && /<li>/.test(html) ? list.parentElement.tagName.toLowerCase() : '';
    document.execCommand('insertHTML', false, tag ? `<${tag}>${html}</${tag}>` : html);
  });

  // The address in front of the caret, while it is plain text: { node, start, end, text, href, tail }. tail: how
  // many characters sit between its end and the caret.
  const typedAtCaret = () => {
    const sel = window.getSelection();
    if (!sel.rangeCount || !sel.isCollapsed) return null;
    const node = sel.anchorNode;
    if (!node || node.nodeType !== Node.TEXT_NODE || !editor.contains(node) || within(node, NO_LINK)) return null;
    const url = typedUrl(node.data.slice(0, sel.anchorOffset));
    return url && { node, ...url, tail: sel.anchorOffset - url.end };
  };
  // undoable: the link typing made last, while Ctrl+Z would take just that link back: { text, skip }.
  let undoable = null;
  function linkTyped(found, after) {
    const { node, start, end, text, href } = found;
    if (!node.isConnected || node.data.slice(start, end) !== text || within(node, NO_LINK)) return;
    const sel = window.getSelection();
    // A range follows the text as the command splits it, so the caret returns to where typing left it.
    const back = sel.rangeCount ? sel.getRangeAt(0).cloneRange() : null;
    const range = document.createRange();
    range.setStart(node, start);
    range.setEnd(node, end);
    select(range);
    document.execCommand('createLink', false, href);
    if (back && editor.contains(back.startContainer)) select(back);
    else sel.collapseToEnd();
    undoable = { text, skip: found.tail + after };
    hush();
  }

  // pending: the address a space or Enter is about to end. The link is made once the space is in, as a step of
  // its own, so Ctrl+Z takes back the link and keeps the text.
  let pending = null;
  listen(editor, 'beforeinput', (e) => {
    pending = null;
    if (e.isComposing) return;
    const ends = e.inputType === 'insertParagraph' || e.inputType === 'insertLineBreak' || (e.inputType === 'insertText' && /^\s$/.test(e.data || ''));
    if (ends) pending = typedAtCaret();
  });
  listen(editor, 'input', (e) => {
    const found = pending;
    const undone = e.inputType === 'historyUndo' ? undoable : null;
    pending = null;
    undoable = null;
    hush();
    // Chromium's undo of the link selects the address. The caret goes back behind the space instead, where it was.
    if (undone) {
      const sel = window.getSelection();
      if (!sel.isCollapsed && sel.toString() === undone.text && editor.contains(sel.anchorNode)) {
        sel.collapseToEnd();
        for (let i = 0; i < undone.skip; i++) sel.modify('move', 'forward', 'character');
      }
    }
    if (found) linkTyped(found, 1);
  });
  // Tab leaves the text, so the link is made before it does.
  listen(editor, 'keydown', (e) => {
    if (e.key !== 'Tab' || e.ctrlKey || e.metaKey || e.altKey || e.isComposing) return;
    const found = typedAtCaret();
    if (found) linkTyped(found, 0);
  });

  return {
    insertLink: () => {
      const again = closedBy;
      closedBy = null;
      if (!again) insertLink();
    },
    dispose: () => cleanups.forEach((fn) => fn())
  };
}
