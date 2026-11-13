// Images in the editor (rich-editor.js): a click selects one, which frames it with a handle on each corner to drag
// it to a size, and puts a bar under it with Small, Best fit, Original size and Remove.
//
// The size is the image's own width, as an attribute for Outlook and a style for the others, so a draft and the
// sent mail keep it. The frame and the bar sit beside the editor, never in its text. Every change goes through the
// editing commands, so Ctrl+Z takes it back in one step.
import { t } from './i18n.js';
import { esc } from './ui.js';

// Widths in pixels. Best fit is what the reading pane of most mail apps shows without scaling.
export const IMAGE_SMALL = 200;
export const IMAGE_FIT = 560;
const MIN = 24;
const GAP = 6;
const EDGE = 8;

// The image a selection holds when it holds that image and nothing else.
export function soleImage(range) {
  if (!range || range.collapsed || range.startContainer !== range.endContainer) return null;
  const at = range.startContainer;
  if (at.nodeType !== Node.ELEMENT_NODE || range.endOffset !== range.startOffset + 1) return null;
  const node = at.childNodes[range.startOffset];
  return node && node.nodeName === 'IMG' ? node : null;
}

// editor: the contenteditable element. onChange(): the editor's own bookkeeping after an edit.
export function mountEditorImages(editor, { onChange }) {
  const cleanups = [];
  const listen = (el, type, fn, opt) => {
    el.addEventListener(type, fn, opt);
    cleanups.push(() => el.removeEventListener(type, fn, opt));
  };

  // The frame scrolls with the text and is clipped with it. The bar sits in the top layer, so no pane clips it,
  // and right after the editor, so Tab reaches it from the text.
  const frame = document.createElement('div');
  frame.className = 'img-frame';
  frame.hidden = true;
  frame.setAttribute('aria-hidden', 'true');
  frame.innerHTML = ['nw', 'ne', 'sw', 'se'].map((c) => `<span class="img-handle" data-corner="${c}"></span>`).join('');
  const bar = document.createElement('div');
  bar.className = 'img-bubble';
  bar.popover = 'manual';
  bar.setAttribute('role', 'toolbar');
  bar.setAttribute('data-i18n-aria-label', 'composer.image.options');
  bar.setAttribute('aria-label', t('composer.image.options'));
  const button = (a, key) => `<button type="button" data-a="${a}" data-i18n="${key}">${esc(t(key))}</button>`;
  bar.innerHTML = `
    ${button('small', 'composer.image.small')}
    ${button('fit', 'composer.image.bestFit')}
    ${button('original', 'composer.image.original')}
    <span class="sep"></span>
    ${button('remove', 'composer.image.remove')}`;
  editor.after(frame, bar);
  cleanups.push(() => {
    frame.remove();
    bar.remove();
  });

  const editable = (node) => (node && node.nodeName === 'IMG' && editor.contains(node) && node.isContentEditable ? node : null);
  const selectNode = (node) => {
    const range = document.createRange();
    range.selectNode(node);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  };
  const selectedImage = () => {
    const sel = window.getSelection();
    return sel.rangeCount ? editable(soleImage(sel.getRangeAt(0))) : null;
  };
  const focusHere = () => document.activeElement === editor || editor.contains(document.activeElement) || bar.contains(document.activeElement);
  const images = () => [...editor.querySelectorAll('img')];

  // ----- the frame and the bar -----

  // shown: the image the frame is around. drag: the resize under way.
  let shown = null;
  let drag = null;

  const hide = () => {
    shown = null;
    frame.hidden = true;
    if (bar.matches(':popover-open')) bar.hidePopover();
  };
  // The part of the image the panes it scrolls in leave in view, or null.
  const visibleRect = (img) => {
    const r = img.getBoundingClientRect();
    let { left, top, right, bottom } = r;
    for (let el = img.parentElement; el; el = el.parentElement) {
      if (!/auto|scroll|hidden|clip/.test(getComputedStyle(el).overflowY)) continue;
      const c = el.getBoundingClientRect();
      left = Math.max(left, c.left);
      top = Math.max(top, c.top);
      right = Math.min(right, c.right);
      bottom = Math.min(bottom, c.bottom);
    }
    top = Math.max(top, 0);
    bottom = Math.min(bottom, window.innerHeight);
    return right > left && bottom > top ? { left, top, right, bottom } : null;
  };
  // Under the image, or above it where the window has no room below; never past the window's edge.
  const placeBar = (at) => {
    const w = bar.offsetWidth;
    const h = bar.offsetHeight;
    const below = at.bottom + GAP;
    const fits = below + h <= window.innerHeight - EDGE;
    const top = fits || at.top - GAP - h < EDGE ? below : at.top - GAP - h;
    bar.style.left = `${Math.max(EDGE, Math.min(at.left, window.innerWidth - w - EDGE))}px`;
    bar.style.top = `${Math.max(EDGE, Math.min(top, window.innerHeight - h - EDGE))}px`;
  };
  // Which of the sizes the image has now.
  const sizeOf = (img) => {
    const width = Number(img.getAttribute('width')) || parseFloat(img.style.width) || 0;
    if (!width) return 'original';
    const natural = img.naturalWidth || Infinity;
    if (width === Math.min(IMAGE_SMALL, natural)) return 'small';
    if (width === Math.min(IMAGE_FIT, natural)) return 'fit';
    return width === natural ? 'original' : '';
  };
  const place = () => {
    const img = shown;
    if (!img) return;
    frame.hidden = false;
    const r = img.getBoundingClientRect();
    const parent = frame.offsetParent || document.body;
    const p = parent.getBoundingClientRect();
    frame.style.left = `${r.left - p.left - parent.clientLeft + parent.scrollLeft}px`;
    frame.style.top = `${r.top - p.top - parent.clientTop + parent.scrollTop}px`;
    frame.style.width = `${r.width}px`;
    frame.style.height = `${r.height}px`;
    const seen = drag ? null : visibleRect(img);
    if (!seen) {
      if (bar.matches(':popover-open')) bar.hidePopover();
      return;
    }
    const size = sizeOf(img);
    for (const b of bar.querySelectorAll('[data-a]')) {
      if (b.dataset.a !== 'remove') b.setAttribute('aria-pressed', String(b.dataset.a === size));
    }
    if (!bar.matches(':popover-open')) bar.showPopover();
    placeBar(seen);
  };
  const sync = () => {
    if (drag) return;
    const img = focusHere() ? selectedImage() : null;
    if (!img) return hide();
    shown = img;
    place();
  };

  // ----- changing the image -----

  // The image with another width, or its own size for null. The command replaces the element, so the new one is
  // found by its place among the images.
  function resize(img, width) {
    const index = images().indexOf(img);
    const next = img.cloneNode();
    next.removeAttribute('height');
    next.style.height = 'auto';
    next.style.maxWidth = '100%';
    if (width) {
      next.setAttribute('width', String(width));
      next.style.width = `${width}px`;
    } else {
      next.removeAttribute('width');
      next.style.width = '';
    }
    editor.focus();
    selectNode(img);
    document.execCommand('insertHTML', false, next.outerHTML);
    const now = images()[index];
    if (now) selectNode(now);
    onChange();
    sync();
  }
  function remove(img) {
    editor.focus();
    selectNode(img);
    document.execCommand('delete');
    onChange();
    sync();
  }
  const preset = (img, name) => {
    const natural = img.naturalWidth || Infinity;
    if (name === 'small') return Math.min(IMAGE_SMALL, natural);
    if (name === 'fit') return Math.min(IMAGE_FIT, natural);
    return null;
  };

  listen(editor, 'click', (e) => {
    const img = editable(e.target);
    if (img) selectNode(img);
  });
  listen(document, 'selectionchange', sync);
  listen(editor, 'input', sync);
  listen(editor, 'focus', sync);
  listen(editor, 'focusout', (e) => {
    if (!bar.contains(e.relatedTarget)) hide();
  });
  listen(bar, 'focusout', (e) => {
    if (e.relatedTarget !== editor && !bar.contains(e.relatedTarget)) hide();
  });
  // An image that finishes loading, a window that changes size and text that scrolls all move the image.
  listen(editor, 'load', place, true);
  listen(document, 'scroll', place, true);
  const watch = new ResizeObserver(place);
  watch.observe(editor);
  cleanups.push(() => watch.disconnect());

  // The editor keeps the focus and the selection when the bar is clicked.
  listen(bar, 'mousedown', (e) => e.preventDefault());
  listen(bar, 'click', (e) => {
    const b = e.target.closest('button');
    const img = shown;
    if (!b || !img) return;
    if (!img.isConnected) return hide();
    if (b.dataset.a === 'remove') remove(img);
    else resize(img, preset(img, b.dataset.a));
  });
  listen(bar, 'keydown', (e) => {
    const buttons = [...bar.querySelectorAll('button')];
    const i = buttons.indexOf(document.activeElement);
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      editor.focus();
      window.getSelection().collapseToEnd();
    } else if ((e.key === 'ArrowRight' || e.key === 'ArrowLeft') && i >= 0) {
      e.preventDefault();
      buttons[(i + (e.key === 'ArrowRight' ? 1 : -1) + buttons.length) % buttons.length].focus();
    }
  });
  listen(editor, 'keydown', (e) => {
    if (e.key !== 'Escape' || !shown || drag) return;
    // Only the image lets go, the composer stays open.
    e.preventDefault();
    e.stopPropagation();
    window.getSelection().collapseToEnd();
  });

  // ----- dragging a corner -----

  const endDrag = (apply) => {
    if (!drag) return;
    const { img, style, width, handle, id, from } = drag;
    drag = null;
    frame.classList.remove('dragging');
    if (handle.hasPointerCapture(id)) handle.releasePointerCapture(id);
    // The drag only showed the size. The image goes back as it was, and the command then sets the size.
    if (style === null) img.removeAttribute('style');
    else img.setAttribute('style', style);
    if (apply && width !== from && img.isConnected) resize(img, width);
    else sync();
  };
  listen(frame, 'pointerdown', (e) => {
    const handle = e.target.closest('.img-handle');
    const img = shown;
    if (!handle || !img || e.button !== 0) return;
    e.preventDefault();
    handle.setPointerCapture(e.pointerId);
    const from = Math.round(img.getBoundingClientRect().width);
    drag = { img, handle, id: e.pointerId, x: e.clientX, from, width: from, style: img.getAttribute('style'), sign: handle.dataset.corner.endsWith('e') ? 1 : -1 };
    frame.classList.add('dragging');
    if (bar.matches(':popover-open')) bar.hidePopover();
  });
  listen(frame, 'pointermove', (e) => {
    if (!drag || e.pointerId !== drag.id) return;
    // The page's zoom scales the pointer's pixels and the image's alike.
    const width = Math.round(Math.max(MIN, Math.min(drag.from + drag.sign * (e.clientX - drag.x), editor.clientWidth)));
    drag.width = width;
    drag.img.style.width = `${width}px`;
    drag.img.style.height = 'auto';
    place();
  });
  listen(frame, 'pointerup', (e) => {
    if (drag && e.pointerId === drag.id) endDrag(true);
  });
  listen(frame, 'pointercancel', () => endDrag(false));
  listen(document, 'keydown', (e) => {
    if (!drag || e.key !== 'Escape') return;
    e.preventDefault();
    e.stopPropagation();
    endDrag(false);
  }, true);

  return { dispose: () => cleanups.forEach((fn) => fn()) };
}
