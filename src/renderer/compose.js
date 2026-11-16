import { t, setLanguage } from './i18n.js';
// The compose window: the editor from composer.js on its own, for mailto links and for messages
// popped out of the main window. It talks to the main process directly.
import { api, esc, toast, applyTheme, onSystemThemeChange } from './ui.js';
import { mountComposer } from './composer.js';

// The window's title bar and the custom title in it.
function setTitle(text) {
  document.title = text;
  document.getElementById('compose-title').textContent = text;
}

async function init() {
  const host = document.getElementById('app');
  const [data, opts] = await Promise.all([api('state'), api('composeInit')]);
  applyTheme(data.settings.theme);
  if (!opts) return api('composeClose');
  // A mailto: link or the tray's New message before any account was added: say why there is no editor. It follows
  // the language and theme like the editor, and turns into the editor, with the link's fields, once an account exists.
  if (!data.accounts.length) {
    const draw = () => {
      setTitle(t('composer.titles.new'));
      host.innerHTML = `<div class="compose-error"><p data-compose-empty>${esc(t('composer.errors.noAccount'))}</p><button class="btn secondary">${esc(t('common.actions.close'))}</button></div>`;
      host.querySelector('button').addEventListener('click', () => api('composeClose'));
    };
    const update = async () => {
      const next = await api('state');
      if (next.accounts.length) return location.reload();
      setLanguage(next.settings.language);
      applyTheme(next.settings.theme);
      draw();
    };
    draw();
    window.mail.on(({ type }) => (type === 'theme' || type === 'language') && update());
    onSystemThemeChange(update);
    // An account is added in the main window; coming back here finds it.
    window.addEventListener('focus', update);
    return;
  }
  // An editor that opens again after Undo send keeps the email it answers, which may have moved meanwhile.
  let message = opts.restore ? opts.restore.original || null : null;
  if (opts.id && !opts.restore) {
    try {
      message = await api('get', opts.id);
    } catch (err) {
      host.innerHTML = `<div class="compose-error"><p class="error">${esc(err.message)}</p><button class="btn secondary" data-i18n="common.actions.close">${esc(t('common.actions.close'))}</button></div>`;
      host.querySelector('button').addEventListener('click', () => api('composeClose'));
      return;
    }
  }
  const editor = mountComposer(host, {
    data,
    opts,
    message,
    inline: false,
    onTitle: setTitle
  });
  const retheme = async () => {
    const next = await api('state');
    setLanguage(next.settings.language);
    applyTheme(next.settings.theme);
    editor.retheme(next);
  };
  window.mail.on(({ type }) => (type === 'theme' || type === 'language') && retheme());
  onSystemThemeChange(retheme);
  // Main's account updates reach only the main window. Coming back from it, the editor follows a default account
  // changed there.
  window.addEventListener('focus', () => api('state').then((next) => editor.accountsChanged(next)).catch(() => {}));
}

init().catch((err) => toast(err.message, 6000));
