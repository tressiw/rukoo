import { t } from './i18n.js';
// Signatures in Settings: the preview in a row, and the dialog to write or paste one. Which signature goes under
// a mail is decided in composer.js.
// The main process cleans every signature it stores (signature.js there); the dialog asks it to clean pasted
// HTML too, so the editor shows what will be saved.
import { icons } from './icons.js';
import { api, esc, toast, dialog } from './ui.js';
import { formatButtons } from './composer.js';

const IMAGE_TYPES = /^image\/(png|jpeg|gif|webp)$/;

// The signature as it will be sent, for a Settings row. Inert: its links are for the recipient, not for Settings.
export const signaturePreview = (html) => `<div class="sig-preview sig-render" inert>${html}</div>`;

const toolbar = () => `
  ${formatButtons()}
  <span class="sep"></span>
  <button class="icon-btn sm" type="button" data-color="foreColor" data-i18n-title="composer.format.textColor" title="${esc(t('composer.format.textColor'))}">${icons.textColor}</button>
  <button class="icon-btn sm" type="button" data-c="image" data-i18n-title="composer.attachments.image" title="${esc(t('composer.attachments.image'))}">${icons.image}</button>
  <span class="spacer"></span>
  <button class="icon-btn sm" type="button" data-c="source" aria-pressed="false" data-i18n-title="settings.signature.source" title="${esc(t('settings.signature.source'))}">${icons.code}</button>`;

const readAsDataUrl = (file) =>
  new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });

// Opens the editor on html. title: whose signature it is. buttons: extra dialog buttons before Cancel and Save, such
// as "Use general signature". Resolves to { html } on Save (cleaned again by main when stored), to an extra button's
// value, or null.
export function editSignature({ html, title = t('settings.general.signature'), buttons = [] }) {
  let sourceMode = false;
  return dialog({
    title,
    wide: true,
    buttons: [
      ...buttons,
      { get label() { return t('common.actions.cancel'); }, value: null },
      {
        get label() { return t('common.actions.save'); },
        primary: true,
        value: (scrim) => ({ html: sourceMode ? scrim.querySelector('.sig-source').value : scrim.querySelector('.sig-editor').innerHTML })
      }
    ],
    render(body) {
      body.innerHTML = `
        <div class="sig-dialog">
          <p class="sig-help" data-i18n="settings.signature.help">${esc(t('settings.signature.help'))}</p>
          <div class="sig-frame">
            <div class="sig-toolbar" role="toolbar" data-i18n-aria-label="composer.format.label" aria-label="${esc(t('composer.format.label'))}">${toolbar()}</div>
            <div class="sig-linkbar" hidden>
              <input type="text" class="sig-link-input" placeholder="https://" spellcheck="false" data-i18n-aria-label="composer.link.address" aria-label="${esc(t('composer.link.address'))}"/>
              <button class="btn sm secondary" type="button" data-c="link-insert" data-i18n="composer.link.insert">${esc(t('composer.link.insert'))}</button>
            </div>
            <div class="sig-editor sig-render" contenteditable="true" spellcheck="true" role="textbox" aria-multiline="true" data-i18n-aria-label="settings.general.signature" aria-label="${esc(t('settings.general.signature'))}" data-i18n-data-placeholder="settings.signature.placeholder" data-placeholder="${esc(t('settings.signature.placeholder'))}"></div>
            <textarea class="sig-source" spellcheck="false" hidden data-i18n-aria-label="settings.signature.source" aria-label="${esc(t('settings.signature.source'))}"></textarea>
          </div>
          <input type="color" class="color-input" value="#2f63c8" tabindex="-1" aria-hidden="true"/>
        </div>`;
      const editor = body.querySelector('.sig-editor');
      const source = body.querySelector('.sig-source');
      const linkBar = body.querySelector('.sig-linkbar');
      const linkInput = body.querySelector('.sig-link-input');
      const sourceBtn = body.querySelector('[data-c="source"]');
      // Parsed as an inert template, then moved in.
      const setHtml = (value) => {
        const tpl = document.createElement('template');
        tpl.innerHTML = value;
        editor.replaceChildren(tpl.content);
      };
      setHtml(html || '');
      document.execCommand('styleWithCSS', false, true);

      let range = null;
      const keepRange = () => {
        const sel = window.getSelection();
        if (sel.rangeCount && editor.contains(sel.anchorNode)) range = sel.getRangeAt(0).cloneRange();
      };
      const restoreRange = () => {
        editor.focus();
        if (!range) return;
        const sel = window.getSelection();
        sel.removeAllRanges();
        sel.addRange(range);
      };
      const insertHtml = (value) => {
        restoreRange();
        document.execCommand('insertHTML', false, value);
        keepRange();
      };
      editor.addEventListener('keyup', keepRange);
      editor.addEventListener('mouseup', keepRange);
      editor.addEventListener('input', keepRange);

      const updateToolbar = () => {
        for (const cmd of ['bold', 'italic', 'underline', 'insertOrderedList', 'insertUnorderedList']) {
          const on = document.queryCommandState(cmd);
          body.querySelectorAll(`[data-cmd="${cmd}"]`).forEach((b) => {
            b.classList.toggle('on', on);
            b.setAttribute('aria-pressed', String(on));
          });
        }
      };
      const onSelection = () => {
        if (!body.isConnected) return document.removeEventListener('selectionchange', onSelection);
        if (document.activeElement === editor) updateToolbar();
      };
      document.addEventListener('selectionchange', onSelection);

      // Pasted HTML from another mail app or a web page goes through main's cleaner first; a pasted picture
      // becomes inline image data. Plain text pastes as it is.
      editor.addEventListener('paste', async (e) => {
        const data = e.clipboardData;
        const pasted = data.getData('text/html');
        const image = [...(data.files || [])].find((f) => IMAGE_TYPES.test(f.type));
        if (!pasted && !image) return;
        e.preventDefault();
        keepRange();
        try {
          const clean = pasted ? await api('cleanSignature', pasted) : `<img src="${esc(await readAsDataUrl(image))}">`;
          if (clean) insertHtml(clean);
        } catch (err) {
          toast(err.message, 5000);
        }
      });

      const showSource = async (on) => {
        if (on) {
          source.value = editor.innerHTML;
        } else {
          try {
            setHtml(await api('cleanSignature', source.value));
          } catch (err) {
            return toast(err.message, 5000);
          }
        }
        sourceMode = on;
        source.hidden = !on;
        editor.hidden = on;
        linkBar.hidden = true;
        sourceBtn.classList.toggle('on', on);
        sourceBtn.setAttribute('aria-pressed', String(on));
        body.querySelectorAll('.sig-toolbar button:not([data-c="source"])').forEach((b) => (b.disabled = on));
        (on ? source : editor).focus();
      };

      const insertLink = () => {
        const url = linkInput.value.trim();
        linkBar.hidden = true;
        if (!url) return restoreRange();
        const href = /^(https?:|mailto:|tel:)/i.test(url) ? url : /^[^\s@]+@[^\s@]+$/.test(url) ? `mailto:${url}` : `https://${url}`;
        restoreRange();
        if (range && !range.collapsed) document.execCommand('createLink', false, href);
        else document.execCommand('insertHTML', false, `<a href="${esc(href)}">${esc(url)}</a>`);
        keepRange();
      };
      linkInput.addEventListener('keydown', (e) => {
        if (e.key !== 'Enter') return;
        e.preventDefault();
        insertLink();
      });

      // Keep the editor's selection when a toolbar button is pressed.
      body.querySelector('.sig-toolbar').addEventListener('mousedown', (e) => {
        if (e.target.closest('button')) e.preventDefault();
      });
      body.addEventListener('click', async (e) => {
        const b = e.target.closest('button');
        if (!b || editor.contains(b)) return;
        if (b.dataset.cmd) {
          restoreRange();
          document.execCommand(b.dataset.cmd, false, null);
          keepRange();
          return updateToolbar();
        }
        if (b.dataset.color) {
          const input = body.querySelector('.color-input');
          keepRange();
          input.onchange = () => {
            restoreRange();
            document.execCommand(b.dataset.color, false, input.value);
            keepRange();
          };
          return input.click();
        }
        switch (b.dataset.c) {
          case 'link': {
            keepRange();
            const selected = range ? range.toString() : '';
            linkInput.value = /^(https?:|mailto:)/i.test(selected) ? selected : '';
            linkBar.hidden = false;
            return linkInput.focus();
          }
          case 'link-insert':
            return insertLink();
          case 'image': {
            keepRange();
            const url = await api('pickImage');
            if (url) insertHtml(`<img src="${esc(url)}">`);
            return;
          }
          case 'source':
            return showSource(!sourceMode);
        }
      });
      requestAnimationFrame(() => editor.focus());
    }
  });
}
