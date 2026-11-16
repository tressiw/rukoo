import { t, languages, setLanguage, localize, getLocale } from './i18n.js';
import { icons } from './icons.js';
import { api, brandLogo, esc, toast, dialog, confirmDialog, choiceDialog, numericDate, hhmm, KEEP_OPEN } from './ui.js';
import { icon as panelIcon } from './agent/bui.js';
import {
  ACTIONS,
  shortcutList,
  storedShortcuts,
  SHORTCUT_ICONS,
  SHORTCUT_NAME,
  SHORTCUT_RESERVED,
  SHORTCUTS_MAX,
  SHORTCUT_LABEL_MAX,
  SHORTCUT_PROMPT_MAX
} from './agent/actions.js';
import { signatureHtml, accountSignatureHtml, usesGeneralSignature } from './composer.js';
import { editSignature, signaturePreview } from './signature.js';
import { accountPages, ACCOUNT_VIEWS } from './rukoo-account.js';
import { categoryPages, CATEGORY_VIEWS } from './categories-settings.js';
import { calendarSettingsPages, CALENDAR_VIEWS } from './calendar-settings.js';
import { meetingSettingsPages, MEETING_VIEWS } from './meeting-settings.js';
import { bookingPages, BOOKING_VIEWS } from './booking-settings.js';
import { videoPages, VIDEO_VIEWS } from './video-settings.js';
import { SCHEMES, ACTIONS as KEY_ACTIONS, bindings, display, keyLabel } from './keymap.js';

// Settings > Keyboard shortcuts: the schemes by the name of their app (keymap.js).
const SCHEME_NAMES = { rukoo: 'Rukoo', gmail: 'Gmail', outlook: 'Outlook', apple: 'Apple Mail', thunderbird: 'Thunderbird' };
// Settings > Swipe and quick actions; engine.js checks the same lists.
const SWIPE = ['none', 'read', 'archive', 'delete', 'star', 'move', 'category'];
const QUICK = ['archive', 'read', 'delete', 'star', 'move', 'category'];

const THEMES = [
  { value: 'system', get label() { return t('settings.theme.system'); } },
  { value: 'light', get label() { return t('settings.theme.light'); } },
  { value: 'dark', get label() { return t('settings.theme.dark'); } }
];
const DENSITY = [
  { value: 'standard', get label() { return t('settings.density.standard'); }, get hint() { return t('settings.density.standardHint'); } },
  { value: 'compact', get label() { return t('settings.density.compact'); }, get hint() { return t('settings.density.compactHint'); } }
];
const BADGES = [
  { value: 'new', get label() { return t('settings.badge.new'); } },
  { value: 'unread', get label() { return t('settings.badge.unread'); } },
  { value: 'none', get label() { return t('common.values.none'); } }
];
const INTERVALS = [
  { value: 0, get label() { return t('settings.sync.manual'); } },
  { value: 1, get label() { return t('settings.sync.minute'); } },
  { value: 5, get label() { return t('settings.sync.fiveMinutes'); } },
  { value: 15, get label() { return t('settings.sync.fifteenMinutes'); } },
  { value: 30, get label() { return t('settings.sync.thirtyMinutes'); } },
  { value: 60, get label() { return t('settings.sync.hour'); } }
];
// Settings > General > Undo send: seconds an email waits after Send (main's outbox.js). Main allows the same values
// (engine.js UNDO_SEND_SECONDS).
const UNDO_SEND = [
  { value: 0, get label() { return t('settings.undoSend.off'); } },
  ...[5, 10, 20, 30].map((count) => ({ value: count, get label() { return t('settings.undoSend.seconds', { count }); } }))
];
// Settings > About: which releases the update check offers (main's updates.js).
const CHANNELS = [
  { value: 'stable', get label() { return t('settings.updates.stable'); }, get hint() { return t('settings.updates.stableHint'); } },
  { value: 'nightly', get label() { return t('settings.updates.nightly'); }, get hint() { return t('settings.updates.nightlyHint'); } }
];
const COLORS = [
  { value: '#2fd6c0', get label() { return t('settings.colors.turquoise'); } },
  { value: '#4a7dff', get label() { return t('settings.colors.blue'); } },
  { value: '#ff8a3d', get label() { return t('settings.colors.orange'); } },
  { value: '#c56cf0', get label() { return t('settings.colors.purple'); } },
  { value: '#f5c542', get label() { return t('settings.colors.yellow'); } },
  { value: '#ff5d73', get label() { return t('settings.colors.red'); } },
  { value: '#5ec2ff', get label() { return t('settings.colors.lightBlue'); } }
];
const hideableViews = () => [
  ['vip', t('mailbox.folders.vip')],
  ['starred', t('mailbox.folders.starred')],
  ['saved', t('mailbox.folders.saved')],
  ['drafts', t('mailbox.folders.drafts')],
  ['sent', t('mailbox.folders.sent')],
  ['trash', t('mailbox.folders.trash')],
  ['junk', t('mailbox.folders.junk')],
  ['archive', t('mailbox.folders.archive')]
];

const labelOf = (list, value) => (list.find((x) => x.value === value) || list[0]).label;
const hostOf = (url) => {
  try {
    return new URL(url).host;
  } catch (_) {
    return url;
  }
};

const AGENT_IDS = ['clark', 'claude', 'codex', 'openclaw'];
const AGENT_PRODUCTS = { clark: 'Hermes Agent', claude: 'Claude Code', codex: 'Codex', openclaw: 'OpenClaw' };
const AGENT_STATES = ['starting', 'ready', 'offline', 'unconfigured', 'disabled', 'missing', 'unauthorized', 'unknown'];
const ACCESS = [
  { value: 'ask', get label() { return t('settings.agents.accessAsk'); }, get hint() { return t('settings.agents.accessAskHint'); } },
  { value: 'full', get label() { return t('settings.agents.accessFull'); }, get hint() { return t('settings.agents.accessFullHint'); } }
];
// The permission level a chat has unless it picked its own (main: agents/permission.js). The names are the chat panel's.
const PERMISSION = [
  { value: 'ask', get label() { return t('agent.permission.ask'); }, get hint() { return t('settings.agents.permissionAskHint'); } },
  { value: 'auto', get label() { return t('agent.permission.auto'); }, get hint() { return t('settings.agents.permissionAutoHint'); } },
  { value: 'full', get label() { return t('agent.permission.full'); }, get hint() { return t('settings.agents.permissionFullHint'); } }
];
// Custom agents (main: agents/config.js CUSTOM_PRESETS): the user's own agents on a server that speaks the Runs API,
// such as Booper, or OpenAI's Chat Completions. Their ids start with cx_.
const isCustom = (id) => /^cx_[a-z0-9]{10}$/.test(String(id || ''));
const customOf = (config, id) => (config && Array.isArray(config.custom) && config.custom.find((x) => x.id === id)) || null;
const allAgentIds = (config) => [...AGENT_IDS, ...((config && config.custom) || []).map((x) => x.id)];
// The settings of an agent: its section, or its entry in the custom list.
const agentConf = (config, id) => (isCustom(id) ? customOf(config, id) || {} : (config && config[id]) || {});
// A patch for one agent, in the shape main takes (config.update()).
const agentPatch = (id, fields) => (isCustom(id) ? { custom: { [id]: fields } } : { [id]: fields });
const CUSTOM_PRESETS = ['booper', 'runs', 'openai'];
const productOf = (config, id) => (isCustom(id) ? t(`settings.agents.preset.${(customOf(config, id) || {}).preset || 'runs'}`) : AGENT_PRODUCTS[id]);
const agentName = (config, id) => {
  if (isCustom(id)) return (customOf(config, id) || {}).name || t('settings.agents.removedAgent');
  return id === 'clark' ? (config && config.clark && config.clark.name) || 'Hermes' : id === 'claude' ? 'Claude' : id === 'openclaw' ? 'OpenClaw' : 'Codex';
};
const agentStateOf = (status, id) => {
  const s = status && status[id] ? status[id].state : 'starting';
  return AGENT_STATES.includes(s) ? s : 'unknown';
};
const agentDot = (state) => (state === 'ready' ? 'ready' : state === 'offline' || state === 'unauthorized' ? 'offline' : state === 'starting' ? 'starting' : 'unconfigured');

// When a shortcut shows in a new chat, for its row in Settings.
const shortcutWhen = (s) =>
  s.custom
    ? s.when === 'always'
      ? t('settings.shortcuts.whenAlways')
      : t('settings.shortcuts.whenEmail')
    : s.needsUnsubscribe
      ? t('settings.shortcuts.whenNewsletter')
      : s.needsEmail
        ? t('settings.shortcuts.whenEmail')
        : t('settings.shortcuts.whenNoEmail');

// Edits one shortcut, or makes a new one of the user's (s null). list: all shortcuts, to keep commands unique.
// Resolves to {label, prompt, key, icon, when}, 'restore' (back to Rukoo's own), 'delete', or null.
function shortcutDialog(s, list) {
  const fresh = !s;
  const custom = fresh || s.custom;
  let iconName = custom ? (s && s.icon) || 'sparkle' : s.icon;
  let when = custom ? (s && s.when) || 'email' : null;
  const buttons = [];
  if (s && s.custom) buttons.push({ get label() { return t('common.actions.delete'); }, value: 'delete', danger: true });
  if (s && !s.custom && (s.edited.label || s.edited.prompt)) buttons.push({ get label() { return t('settings.shortcuts.restore'); }, value: 'restore', left: true });
  buttons.push({ get label() { return t('common.actions.cancel'); }, value: null });
  buttons.push({
    get label() {
      return fresh ? t('common.actions.add') : t('common.actions.save');
    },
    value: (scrim) => {
      const form = scrim.querySelector('.shortcut-form');
      const error = form.querySelector('[data-error]');
      const field = (name) => form.querySelector(`[name="${name}"]`);
      const fail = (name, message) => {
        error.textContent = message;
        field(name).focus();
        return KEEP_OPEN;
      };
      const label = field('label').value.trim();
      const prompt = field('prompt').value.trim();
      if (!label) return fail('label', t('settings.shortcuts.needLabel'));
      if (!prompt) return fail('prompt', t('settings.shortcuts.needPrompt'));
      if (!custom) return { label, prompt };
      const key = field('key').value.trim().replace(/^\//, '').toLowerCase();
      if (!SHORTCUT_NAME.test(key) || key.length > 40) return fail('key', t('settings.shortcuts.badCommand'));
      const taken = SHORTCUT_RESERVED.has(key) || ACTIONS.some((a) => a.key === key) || list.some((x) => x.key === key && x !== s);
      if (taken) return fail('key', t('settings.shortcuts.takenCommand', { name: key }));
      return { label, prompt, key, icon: iconName, when };
    }
  });
  return dialog({
    title: fresh ? t('settings.shortcuts.newTitle') : t('settings.shortcuts.editTitle'),
    wide: true,
    buttons,
    render(body) {
      const icons = SHORTCUT_ICONS.map(
        (name) =>
          `<button type="button" class="shortcut-icon" role="radio" data-icon="${name}" aria-checked="${name === iconName}" aria-label="${esc(name)}">${panelIcon(name, 15).outerHTML}</button>`
      ).join('');
      const whenRow = (value, label) =>
        `<button type="button" class="radio-row" role="radio" data-when="${value}" aria-checked="${value === when}"><span class="radio ${value === when ? 'on' : ''}"></span><span class="rl">${esc(label)}</span></button>`;
      body.innerHTML = `<div class="form shortcut-form" style="padding:0">
        <label>${esc(t('settings.shortcuts.label'))}<input type="text" name="label" maxlength="${SHORTCUT_LABEL_MAX}" spellcheck="false" autocomplete="off" value="${esc(s ? s.label : '')}"/></label>
        <label>${esc(t('settings.shortcuts.command'))}<span class="shortcut-command"><span aria-hidden="true">/</span><input type="text" name="key" maxlength="40" spellcheck="false" autocomplete="off" value="${esc(s ? s.key : '')}" ${custom ? '' : 'disabled'} placeholder="${esc(t('settings.shortcuts.commandPlaceholder'))}"/></span>
          <span class="field-help">${esc(custom ? t('settings.shortcuts.commandHelp') : t('settings.shortcuts.commandFixed'))}</span></label>
        ${
          custom
            ? `<div class="shortcut-field"><span class="shortcut-field-label" id="shortcut-when">${esc(t('settings.shortcuts.when'))}</span>
          <div class="choices" role="radiogroup" aria-labelledby="shortcut-when">${whenRow('email', t('settings.shortcuts.whenEmailOption'))}${whenRow('always', t('settings.shortcuts.whenAlwaysOption'))}</div></div>
        <div class="shortcut-field"><span class="shortcut-field-label" id="shortcut-icon">${esc(t('settings.shortcuts.icon'))}</span>
          <div class="shortcut-icons" role="radiogroup" aria-labelledby="shortcut-icon">${icons}</div></div>`
            : ''
        }
        <label>${esc(t('settings.shortcuts.prompt'))}<textarea name="prompt" rows="7" maxlength="${SHORTCUT_PROMPT_MAX}" spellcheck="false">${esc(s ? s.prompt : '')}</textarea>
          <span class="field-help">${esc(custom ? t('settings.shortcuts.promptHelp') : t('settings.shortcuts.promptHelpBuiltin'))}</span></label>
        <div class="error" data-error></div>
      </div>`;
      // An error is about what the field held: it goes once the user changes it.
      body.querySelector('.shortcut-form').addEventListener('input', () => (body.querySelector('[data-error]').textContent = ''));
      body.querySelectorAll('[data-when]').forEach((b) =>
        b.addEventListener('click', () => {
          when = b.dataset.when;
          body.querySelectorAll('[data-when]').forEach((x) => {
            x.setAttribute('aria-checked', String(x === b));
            x.querySelector('.radio').classList.toggle('on', x === b);
          });
        })
      );
      body.querySelectorAll('[data-icon]').forEach((b) =>
        b.addEventListener('click', () => {
          iconName = b.dataset.icon;
          body.querySelectorAll('[data-icon]').forEach((x) => x.setAttribute('aria-checked', String(x === b)));
        })
      );
    }
  });
}

export function promptDialog(title, value = '', { multiline = false, placeholder = '' } = {}) {
  return dialog({
    title,
    buttons: [
      { get label() { return t('common.actions.cancel'); }, value: null },
      { get label() { return t('common.actions.save'); }, value: (scrim) => scrim.querySelector('.form-input').value }
    ],
    render(body) {
      body.innerHTML = `<div class="form" style="padding:0">${
        multiline
          ? `<textarea class="form-input" placeholder="${esc(placeholder)}">${esc(value)}</textarea>`
          : `<input class="form-input" type="text" value="${esc(value)}" placeholder="${esc(placeholder)}"/>`
      }</div>`;
    }
  });
}

// view: open straight on one page (the chat panel opens 'agents'); Back still leads to the overview. An account's
// 'server' page needs the account's id, and Back from it leads to that account.
export function openSettings(ctx, { view: start, id = null } = {}) {
  const page = document.createElement('div');
  page.className = 'page settings';
  document.getElementById('overlays').appendChild(page);
  const stack = !start ? ['main'] : start === 'server' ? ['main', 'account', 'server'] : start === 'shortcuts' ? ['main', 'agents', 'shortcuts'] : ['main', start];
  let params = id ? { id } : {};
  // What the update check found (main's updates.js), and how far its download got (installer.js; null where this
  // install can't update itself), for the Check now row.
  let updates = null;
  const download = () => {
    const install = updates && updates.install;
    return install && updates.available && install.version === updates.available.version ? install : null;
  };
  const updatesValue = () => {
    const d = download();
    if (d && d.status === 'downloading') return t('settings.updates.downloading', { version: d.version, percent: d.percent });
    if (d && d.status === 'ready') return t('settings.updates.ready', { version: d.version });
    if (updates && updates.available) return t('settings.updates.available', { version: updates.available.version });
    return updates && updates.checkedAt ? t('settings.updates.upToDate') : '';
  };
  // Download progress changes only the value; a finished download also brings the Restart row.
  const showUpdates = (state) => {
    const was = download() && download().status;
    updates = state;
    if ((download() && download().status) !== was) return render();
    const value = page.querySelector('[data-a="updates-check"] .value');
    if (value) value.textContent = updatesValue();
  };
  // While the pairing code shows: what the dialog does with the phone events from main (paired, expired, burned).
  let pairWatch = null;
  // The test notification row: whether this system offers Rukoo notifications at all (main's
  // Notification.isSupported()), and what the last test came to (desktop.js testNotice), or testing while one is on its
  // way. Whether the system allows them can't be asked, so the test is how to find out.
  const notices = { supported: null, test: null, testing: false };
  const noticesDesc = () => {
    const r = notices.test;
    if (notices.testing) return t('settings.general.testNotificationSending');
    if ((r && r.result === 'unavailable') || notices.supported === false) return t('settings.general.notificationsUnavailable');
    if (r && r.result === 'sent') return t('settings.general.testNotificationSent');
    if (r && r.result === 'failed') return r.error ? t('settings.general.testNotificationFailedReason', { reason: r.error }) : t('settings.general.testNotificationFailed');
    return t('settings.general.testNotificationHelp');
  };

  // Live status (sync times, errors) matters on the overview pages; forms must not be reset under the user.
  const unsubscribe = window.mail.on(({ type, payload }) => {
    // A phone paired, fetched the export, was removed, or was seen: the dialog and the Phone group follow.
    if (type === 'agent' && payload && payload.kind === 'phone') {
      if (pairWatch) pairWatch(payload);
      if (stack[stack.length - 1] === 'agents' && agentState) refreshPhone();
    }
    const view = stack[stack.length - 1];
    if (type === 'updated' && (view === 'main' || view === 'account')) setTimeout(render, 80);
    // Agent status changes only touch the status lines, never the fields you are typing in.
    if (type === 'agent' && payload && payload.kind === 'agents' && (view === 'agents' || view === 'main') && agentState) {
      agentState.status = payload.status || agentState.status;
      showAgentStatuses(view);
    }
    // Zoom keys and the wheel work with Settings open; only the zoom row changes.
    if (type === 'zoom') showZoom(payload);
    if (type === 'updates' && view === 'main') showUpdates(payload);
    if (type === 'account') account.onStatus(payload);
    if (type === 'categories') categoriesPage.onState(payload);
    // The booking page's state from main: the page and the overview follow it, forms keep what is typed.
    if (type === 'booking') bookingPage.onState(payload);
    if (type === 'video') videoPage.onState(payload);
    if (type === 'categories-progress') categoriesPage.onActivity(payload);
    // Rukoo Voice connected, left or started a meeting: the Meetings row and page say so.
    if (type === 'meeting' && payload && payload.kind === 'state') meetingPage.onState(payload.state);
  });

  const close = () => {
    unsubscribe();
    page.remove();
    document.removeEventListener('keydown', onKey, true);
    window.removeEventListener('focus', showMailto);
  };
  const back = () => {
    if (stack.length > 1) {
      stack.pop();
      render();
    } else close();
  };
  const go = (name, p = {}) => {
    stack.push(name);
    params = p;
    render();
  };
  const onKey = (e) => {
    if (e.key === 'Escape' && !document.querySelector('.scrim')) {
      e.stopPropagation();
      back();
    }
  };
  document.addEventListener('keydown', onKey, true);

  // preview: markup shown under the value, such as a signature as it will be sent.
  const row = ({ title, desc, value, preview, action, toggle, sep, cls = '', lead = '' }) => `
    <button class="row ${cls}" ${action ? `data-a="${action}"` : ''}>
      ${lead}
      <div class="text"><div class="title ${cls.includes('danger') ? 'danger' : ''}">${title}</div>
      ${desc ? `<div class="desc">${desc}</div>` : ''}${value ? `<div class="value">${value}</div>` : ''}${preview || ''}</div>
      ${toggle !== undefined ? `${sep ? '<span class="switch-sep"></span>' : ''}<span class="switch ${toggle ? 'on' : ''}" role="switch" aria-checked="${toggle}"></span>` : ''}
    </button>`;

  // Marks the default account in the overview and the default address on the sender addresses page.
  const defaultBadge = () => `<span class="default-badge">${esc(t('settings.account.defaultBadge'))}</span>`;
  // An account's first letter in its color, in the overview and on the Signatures page.
  const accountAvatar = (a) => `<div class="avatar small" style="--accent:${esc(a.color)}">${esc((a.name || a.email).trim()[0] || '?').toUpperCase()}</div>`;

  // The Rukoo account's pages (rukoo-account.js).
  // Inbox categories (categories-settings.js); the account's privacy page says when Jev is on.
  const categoriesPage = categoryPages({ row, go, rerender: () => render(), current: () => stack[stack.length - 1] });
  const account = accountPages({ row, go, back, rerender: () => render(), current: () => stack[stack.length - 1], promptDialog, jevOn: () => categoriesPage.jevOn() });
  // Settings > Calendar (calendar-settings.js) and Settings > Booking page (booking-settings.js), which also draws the
  // booking link rows beside the signature rows.
  const calendarPage = calendarSettingsPages({ row, go, rerender: () => render(), current: () => stack[stack.length - 1] });
  const bookingPage = bookingPages({ row, go, back, rerender: () => render(), current: () => stack[stack.length - 1], promptDialog });
  // Settings > Meetings (meeting-settings.js): Rukoo Voice and what Rukoo does during a meeting.
  const meetingPage = meetingSettingsPages({ row, go, rerender: () => render(), current: () => stack[stack.length - 1] });
  // Settings > Video meetings (video-settings.js), beside the booking page.
  const videoPage = videoPages({ row, go, rerender: () => render(), current: () => stack[stack.length - 1], promptDialog });
  // A redraw of the screen already showing keeps its scroll position on the pages with calendar, booking and signature
  // rows: the booking page changes while it is open, and a switch or a saved signature near the bottom shouldn't send
  // the page back to the top.
  // Another page, or the booking page's setup becoming the page, starts at the top.
  const scrollKept = { key: null, top: 0 };
  function keepScroll(view) {
    const scroller = page.querySelector('.page-scroll');
    const kept = view === 'main' || view === 'account' || view === 'signatures' || CALENDAR_VIEWS.includes(view) || BOOKING_VIEWS.includes(view);
    const key = !kept || !scroller ? null : BOOKING_VIEWS.includes(view) ? bookingPage.screen() : `${view}:${params.id || ''}`;
    if (key && key === scrollKept.key && scrollKept.top > 0) {
      const top = scrollKept.top;
      scroller.scrollTop = top;
      // Rows that fill in just after the redraw can still make the page long enough.
      if (scroller.scrollTop < top) requestAnimationFrame(() => scroller.isConnected && (scroller.scrollTop = top));
    } else scrollKept.top = 0;
    scrollKept.key = key;
    if (key) scroller.addEventListener('scroll', () => scrollKept.key === key && (scrollKept.top = scroller.scrollTop), { passive: true });
  }

  // The general signature's value and preview: the signature as it will be sent, or None.
  const generalSignatureFields = (s) => {
    const html = signatureHtml(s.signature, s.signatureHtml);
    return { value: html ? '' : esc(t('common.values.none')), preview: html && signaturePreview(html) };
  };
  // What an account sends under its mail: its own signature (own: what the value says then), None, or the general
  // one, which the value names.
  const accountSignatureFields = (a, s, own = '') => {
    if (usesGeneralSignature(a)) {
      const html = signatureHtml(s.signature, s.signatureHtml);
      return {
        value: esc(html ? t('settings.signature.usesGeneral') : t('settings.signature.generalValue', { signature: t('common.values.noneLower') })),
        preview: html && signaturePreview(html)
      };
    }
    const html = accountSignatureHtml(a, s);
    return { value: html ? own : esc(t('common.values.none')), preview: html && signaturePreview(html) };
  };
  // An account's row on its own page, and on its sender addresses page with a title that says which addresses send it.
  const accountSignatureRow = (a, s, title = esc(t('settings.general.signature'))) =>
    row({ title, ...accountSignatureFields(a, s), action: `acc-signature:${encodeURIComponent(a.id)}` });

  // A static row, because the reset button can't sit inside a row button.
  const zoomRow = (level) => `
    <div class="row static" data-zoom>
      <div class="text"><div class="title">${esc(t('settings.general.zoom'))}</div>
        <div class="desc">${esc(t(window.mail.platform === 'darwin' ? 'settings.general.macZoomHelp' : 'settings.general.zoomHelp'))}</div>
        <div class="value" data-zoom-value>${esc(t('settings.general.zoomValue', { percent: level }))}</div></div>
      <button class="btn sm secondary" data-a="zoom-reset" ${level === 100 ? 'disabled' : ''}>${esc(t('settings.general.zoomReset'))}</button>
    </div>`;
  const showZoom = (level) => {
    const value = page.querySelector('[data-zoom-value]');
    if (!value || typeof level !== 'number') return;
    value.textContent = t('settings.general.zoomValue', { percent: level });
    page.querySelector('[data-a="zoom-reset"]').disabled = level === 100;
  };

  // Which app opens mailto: links: { available, app, rukoo } from main, or null until the desktop has answered.
  let mailtoState = null;
  const mailtoKey = (kind) => {
    const platform = mailtoState?.platform || (mailtoState?.linux ? 'linux' : window.mail.platform);
    const prefix = platform === 'darwin' ? 'mailtoMac' : platform === 'linux' ? 'mailtoLinux' : 'mailto';
    return `settings.general.${prefix}${kind}`;
  };
  const mailtoValue = () => {
    if (!mailtoState) return '';
    if (!mailtoState.available) return t(mailtoKey('Unavailable'));
    if (mailtoState.rukoo) return t('settings.general.mailtoRukoo');
    return mailtoState.app ? t('settings.general.mailtoHandler', { app: mailtoState.app }) : t('settings.general.mailtoNone');
  };
  const mailtoDisabled = () => !mailtoState || !mailtoState.available || mailtoState.rukoo;
  // A static row like the zoom row. The page draws with the last answer; showMailto() fills in the new one.
  const mailtoRow = () => `
    <div class="row static" data-mailto>
      <div class="text"><div class="title">${esc(t('settings.general.mailto'))}</div>
        <div class="desc" data-mailto-help>${esc(t(mailtoKey('Help')))}</div>
        <div class="value" data-mailto-value>${esc(mailtoValue())}</div></div>
      <button class="btn sm secondary" data-a="mailto-use" ${mailtoDisabled() ? 'disabled' : ''}>${esc(t('settings.general.mailtoUse'))}</button>
    </div>`;
  const showMailto = async () => {
    const next = await api('mailtoStatus').catch(() => null);
    if (!next) return;
    mailtoState = next;
    const value = page.querySelector('[data-mailto-value]');
    if (!value) return;
    value.textContent = mailtoValue();
    page.querySelector('[data-mailto-help]').textContent = t(mailtoKey('Help'));
    page.querySelector('[data-a="mailto-use"]').disabled = mailtoDisabled();
  };
  // Coming back from desktop settings shows the current default handler.
  window.addEventListener('focus', showMailto);

  // ----- agents -----

  // { config, status } as shown on the agents page; config never holds secrets.
  let agentState = null;
  // Per agent with a secret (clark, openclaw): whether its field is open for a new one.
  let keyEditing = {};
  // The config is local and quick. Status can wait on the network (an offline Hermes server takes seconds),
  // so pages draw with the last known status and refreshAgentStatus() fills it in afterwards.
  const loadAgents = () =>
    api('agentConfig')
      .then((config) => ({ config, status: (agentState && agentState.status) || null }))
      .catch(() => null);
  const agentsRowDesc = () =>
    t('settings.agents.rowDesc', { name: esc(agentName(agentState.config, agentState.config.defaultAgent)), status: t(`agent.status.${agentStateOf(agentState.status, agentState.config.defaultAgent)}`) });
  const showAgentStatuses = (view) => {
    if (!agentState) return;
    if (view === 'agents') allAgentIds(agentState.config).forEach((id) => showAgentStatus(id));
    const desc = view === 'main' && page.querySelector('[data-a="agents"] .desc');
    if (desc) desc.innerHTML = agentsRowDesc();
  };
  const refreshAgentStatus = (view) =>
    api('agentStatus')
      .then((status) => {
        if (!agentState || stack[stack.length - 1] !== view) return;
        agentState.status = status;
        showAgentStatuses(view);
      })
      .catch(() => {});
  // The agent's name appears in several labels; this span lets a rename update them in place.
  const nameSpan = (id) => `<span data-agent-name="${id}">${esc(agentName(agentState.config, id))}</span>`;

  const statusRow = (id) => `
    <div class="row static agent-status" data-agent-status="${id}">
      <span class="agent-dot" aria-hidden="true"></span>
      <div class="text"><div class="title"></div><div class="desc agent-detail"></div></div>
      ${id === 'clark' || id === 'openclaw' || isCustom(id)
        ? `<button class="btn sm secondary" data-a="agent-test:${id}" data-i18n="settings.agents.testConnection">${esc(t('settings.agents.testConnection'))}</button>`
        : `<button class="btn sm secondary" data-a="agent-test:${id}" data-i18n="settings.agents.test">${esc(t('settings.agents.test'))}</button>`}
    </div>`;
  function showAgentStatus(id, override) {
    const el = page.querySelector(`[data-agent-status="${id}"]`);
    if (!el || !agentState) return;
    const s = override || (agentState.status && agentState.status[id]) || {};
    const state = override && override.testing ? 'testing' : agentStateOf(agentState.status, id);
    el.querySelector('.agent-dot').dataset.state = state === 'testing' ? 'starting' : agentDot(state);
    el.querySelector('.title').textContent = state === 'testing' ? t('settings.agents.testing') : t(`agent.status.${state}`);
    el.querySelector('.agent-detail').textContent = state === 'testing' ? '' : String(s.detail || '');
    el.querySelector('button').disabled = state === 'testing';
  }

  const agentField = (id, key, label, value, placeholder) =>
    `<label>${esc(label)}<input type="text" data-agent-field="${id}.${key}" value="${esc(value || '')}" placeholder="${esc(placeholder || '')}" spellcheck="false" autocomplete="off"/></label>`;
  // Where the program was found, when main says so; otherwise what it looks for.
  const exeHint = (id, exe) => {
    const detail = String((agentState.status && agentState.status[id] && agentState.status[id].detail) || '');
    const found = agentState.config[id].detectedExe || (/[\\/].+\.exe$/i.test(detail) ? detail : '');
    return found || t('settings.agents.exeHint', { exe });
  };
  // The secret's labels: Hermes has an API key, OpenClaw a gateway token (or password).
  const KEY_TEXT = {
    clark: { label: 'settings.agents.key', placeholder: 'settings.agents.keyPlaceholder', saved: 'settings.agents.keySaved', save: 'settings.agents.keySave', show: 'settings.agents.keyShow', hide: 'settings.agents.keyHide', stored: 'settings.agents.keyStored' },
    openclaw: { label: 'settings.agents.token', placeholder: 'settings.agents.tokenPlaceholder', saved: 'settings.agents.tokenSaved', save: 'settings.agents.tokenSave', show: 'settings.agents.tokenShow', hide: 'settings.agents.tokenHide', stored: 'settings.agents.tokenStored' }
  };
  const keyText = (id) => KEY_TEXT[isCustom(id) ? 'clark' : id];
  const keyHtml = (id) => {
    const k = keyText(id);
    const saved = agentConf(agentState.config, id).hasKey;
    if (saved && !keyEditing[id]) {
      return `<div class="agent-key"><span class="agent-key-label">${esc(t(k.label))}</span>
        <span class="agent-key-saved">${icons.check}<span>${esc(t(k.saved))}</span></span>
        <button class="link-btn" data-a="agent-key-replace:${id}">${esc(t('settings.agents.keyReplace'))}</button></div>`;
    }
    return `<label>${esc(t(k.label))}<span class="pw"><input type="password" data-agent-key="${id}" autocomplete="off" spellcheck="false" placeholder="${esc(t(k.placeholder))}"/><button class="icon-btn sm" data-a="agent-key-reveal:${id}" title="${esc(t(k.show))}">${icons.eye}</button></span></label>
      <div class="agent-key-actions"><button class="btn sm" data-a="agent-key-save:${id}">${esc(t(k.save))}</button>${saved ? `<button class="link-btn" data-a="agent-key-cancel:${id}">${esc(t('common.actions.cancel'))}</button>` : ''}</div>`;
  };
  // The port the remote listener really got (it moves when the chosen one is taken), the same as in the
  // copied Hermes setup, with main's note about the move. Clark signs in with the API key, so without one it can't.
  const remoteDesc = () => {
    const { mcp = {}, clark = {} } = agentState.config;
    const url = mcp.address ? esc(`http://${mcp.address}:${mcp.remotePort || mcp.port || mcp.localPort}/mcp`) : esc(t('settings.agents.noAddress'));
    const note = mcp.remote && !clark.hasKey ? t('settings.agents.remoteNeedsKey', { name: agentName(agentState.config, 'clark') }) : mcp.note;
    return note ? `${url}<span class="agent-note">${esc(note)}</span>` : url;
  };

  // The OpenClaw card's remote row: off, what it is for; on, the address a remote gateway uses (the Tailscale
  // listener's real port, as in the copied setup), with main's note. OpenClaw's token comes from the gateway token.
  const openclawRemoteDesc = () => {
    const { mcp = {}, openclaw = {} } = agentState.config;
    if (!openclaw.remote) return esc(t('settings.agents.openclawRemoteHelp'));
    const url = mcp.address && mcp.remotePort ? esc(`http://${mcp.address}:${mcp.remotePort}/mcp`) : esc(t('settings.agents.noAddress'));
    const note = !openclaw.hasKey ? t('settings.agents.openclawRemoteNeedsToken') : mcp.note;
    return note ? `${url}<span class="agent-note">${esc(note)}</span>` : url;
  };
  // A custom agent's remote row, as OpenClaw's: off, what it is for; on, the address the agent uses.
  const customRemoteDesc = (id) => {
    const { mcp = {} } = agentState.config;
    const x = customOf(agentState.config, id) || {};
    if (!x.remote) return esc(t('settings.agents.customRemoteHelp', { name: x.name || '' }));
    const url = mcp.address && mcp.remotePort ? esc(`http://${mcp.address}:${mcp.remotePort}/mcp`) : esc(t('settings.agents.noAddress'));
    return mcp.note ? `${url}<span class="agent-note">${esc(mcp.note)}</span>` : url;
  };
  // Which setup the copy row puts on the clipboard (a token line and a command): for this computer, or for the
  // gateway's computer.
  const openclawCopyDesc = () => t(agentState.config.openclaw.remote ? 'settings.agents.copyOpenClawRemoteHelp' : 'settings.agents.copyOpenClawHelp');
  // Both cards show the Tailscale listener; a toggle on one can open or close it for the other.
  const showAddresses = () => {
    page.querySelectorAll('[data-custom-address]').forEach((el) => (el.innerHTML = customRemoteDesc(el.dataset.customAddress)));
    const hermes = page.querySelector('[data-agent-address]');
    if (hermes) hermes.innerHTML = remoteDesc();
    const openclaw = page.querySelector('[data-openclaw-address]');
    if (openclaw) openclaw.innerHTML = openclawRemoteDesc();
    const copy = page.querySelector('[data-a="agent-copy-openclaw"] .desc');
    if (copy) copy.textContent = openclawCopyDesc();
    const phone = page.querySelector('[data-phone-address]');
    if (phone) phone.innerHTML = phoneDesc();
  };

  // The Phone group. On: the address a phone pairs with (the Tailscale listener's real port), with main's note.
  const phoneDesc = () => {
    const { mcp = {}, phone = {} } = agentState.config;
    if (!phone.enabled) return esc(t('settings.phone.hostHelp'));
    const url = mcp.address && mcp.remotePort ? esc(`http://${mcp.address}:${mcp.remotePort}`) : esc(t('settings.agents.noAddress'));
    return mcp.note ? `${url}<span class="agent-note">${esc(mcp.note)}</span>` : url;
  };
  const phoneSeen = (p) =>
    [p.lastSeenAt ? t('settings.phone.lastSeen', { date: numericDate(p.lastSeenAt), time: hhmm(p.lastSeenAt) }) : t('settings.phone.neverSeen'), p.exportOpen ? t('settings.phone.exportPending') : '']
      .filter(Boolean)
      .join(' · ');
  const phoneRows = () => {
    const phone = agentState.config.phone || { enabled: false, pairings: [] };
    return `${row({ title: esc(t('settings.phone.host')), desc: `<span data-phone-address>${phoneDesc()}</span>`, action: 'agent-phone', toggle: Boolean(phone.enabled) })}
      ${(phone.pairings || [])
        .map((p) => row({ title: esc(p.name), desc: esc(phoneSeen(p)), action: `agent-phone-manage:${esc(p.id)}`, lead: `<span class="phone-ic">${icons.phone}</span>`, cls: 'indent' }))
        .join('')}
      ${row({ title: esc(t('settings.phone.pair')), desc: esc(t('settings.phone.pairHelp')), action: 'agent-phone-pair', lead: `<span class="add-ic">${icons.plus}</span>`, cls: 'indent' })}`;
  };
  async function refreshPhone() {
    try {
      agentState.config = await api('agentConfig');
    } catch (_) {
      return;
    }
    const card = page.querySelector('[data-phone-card]');
    if (card) card.innerHTML = phoneRows();
    showAddresses();
  }

  // The code, this computer's address and the time left. Resolves {name} once the phone paired, null otherwise.
  function pairDialog(shown) {
    let timer = null;
    return dialog({
      title: t('settings.phone.pairTitle'),
      buttons: [{ get label() { return t('common.actions.cancel'); }, value: null }],
      render(body, done) {
        body.innerHTML = `<p>${esc(t('settings.phone.pairSteps'))}</p>
          <dl class="phone-pair">
            <dt>${esc(t('settings.phone.address'))}</dt><dd class="phone-pair-address">${esc(shown.url)}</dd>
            <dt>${esc(t('settings.phone.code'))}</dt><dd class="phone-pair-code" data-phone-code>${esc(shown.code)}</dd>
          </dl>
          <p class="phone-pair-time" data-phone-time aria-live="polite"></p>
          ${shown.allowExport ? `<p class="phone-pair-note">${esc(t('settings.phone.pairExportNote'))}</p>` : ''}`;
        const time = body.querySelector('[data-phone-time]');
        // The code no longer works: say why, and Cancel becomes Close.
        const end = (text) => {
          clearInterval(timer);
          body.querySelector('[data-phone-code]').classList.add('gone');
          time.textContent = text;
          time.classList.add('ended');
          const button = body.parentElement.querySelector('.buttons button');
          if (button) button.textContent = t('common.actions.close');
        };
        const tick = () => {
          const left = shown.expiresAt - Date.now();
          if (left <= 0) return end(t('settings.phone.pairExpired'));
          time.textContent = t('settings.phone.pairExpires', { count: Math.ceil(left / 60000) });
        };
        timer = setInterval(tick, 1000);
        tick();
        pairWatch = (p) => {
          if (p.event === 'paired') done({ name: (p.pairing && p.pairing.name) || '' });
          else if (p.event === 'burned') end(t('settings.phone.pairBurned'));
          else if (p.event === 'expired') end(t('settings.phone.pairExpired'));
        };
      }
    }).finally(() => {
      clearInterval(timer);
      pairWatch = null;
    });
  }

  async function pairPhone() {
    const choice = await choiceDialog(t('settings.phone.pair'), [
      { value: 'export', label: t('settings.phone.pairWithExport'), hint: t('settings.phone.pairWithExportHint') },
      { value: 'only', label: t('settings.phone.pairOnly'), hint: t('settings.phone.pairOnlyHint') }
    ], null, { stacked: true });
    if (!choice) return;
    let shown;
    try {
      shown = await api('agentPhonePair', { allowExport: choice === 'export' });
    } catch (err) {
      await refreshPhone();
      return toast(err.message === 'not-configured' ? t('settings.phone.noListener') : err.message, 6000);
    }
    // Pairing turns the switch on.
    await refreshPhone();
    const paired = await pairDialog(shown);
    if (paired) toast(t('settings.phone.paired', { name: paired.name }), 4000);
    else api('agentPhonePairCancel').catch(() => {});
    refreshPhone();
  }

  async function managePhone(id) {
    const p = ((agentState.config.phone && agentState.config.phone.pairings) || []).find((x) => x.id === id);
    if (!p) return;
    const choice = await choiceDialog(p.name, [
      { value: 'export', label: t('settings.phone.exportAgain'), hint: t('settings.phone.pairWithExportHint') },
      { value: 'remove', label: t('settings.phone.remove') }
    ], null, { stacked: true });
    if (choice === 'export') {
      try {
        await api('agentPhoneAllowExport', id);
        toast(t('settings.phone.exportOpen'), 5000);
      } catch (err) {
        toast(err.message, 5000);
      }
      return refreshPhone();
    }
    if (choice !== 'remove') return;
    if (!(await confirmDialog(t('settings.phone.remove'), t('settings.phone.removeConfirm', { name: p.name }), t('settings.phone.removeOk'), true))) return;
    try {
      await api('agentPhoneUnpair', id);
      toast(t('settings.phone.removed', { name: p.name }), 4000);
    } catch (err) {
      toast(err.message, 5000);
    }
    return refreshPhone();
  }

  function agentsHtml() {
    const c = agentState.config;
    const accessRow = (id) => row({ title: t('settings.agents.access'), value: esc(labelOf(ACCESS, c[id].access)), action: `agent-access:${id}` });
    // The product beside the name ("Clark  Hermes Agent"), unless they are the same word.
    const head = (id) =>
      `<div class="settings-group-title agent-group">${nameSpan(id)}${productOf(c, id) !== agentName(c, id) ? `<span class="agent-product">${esc(productOf(c, id))}</span>` : ''}</div>`;
    // A custom agent's card: the server, its key and model, Rukoo's tools for it, and Remove.
    const customCard = (x) => {
      const id = x.id;
      const placeholder = x.preset === 'booper' ? 'http://100.64.0.1:8662' : x.protocol === 'openai' ? 'http://127.0.0.1:8000' : 'http://100.64.0.1:8642';
      return `${head(id)}
      <div class="card agent-card" data-custom-card="${id}">
        ${statusRow(id)}
        ${row({ title: t('settings.agents.enabled', { name: nameSpan(id) }), action: `agent-enabled:${id}`, toggle: x.enabled !== false })}
        <div class="form agent-form">
          ${agentField(id, 'name', t('settings.agents.name'), x.name, t(`settings.agents.preset.${x.preset}`))}
          ${agentField(id, 'url', t('settings.agents.url'), x.url, placeholder)}
          <div class="agent-key-box" data-agent-key-box="${id}">${keyHtml(id)}</div>
          ${agentField(id, 'model', t('settings.agents.model'), x.model, t('settings.agents.optional'))}
        </div>
        ${x.protocol === 'openai' ? row({ title: t('settings.agents.customHistory'), desc: esc(t('settings.agents.customHistoryHelp')), action: `agent-custom-history:${id}`, toggle: x.history !== false }) : ''}
        ${row({ title: t('settings.agents.customRemote', { name: nameSpan(id) }), desc: `<span data-custom-address="${id}">${customRemoteDesc(id)}</span>`, action: `agent-custom-remote:${id}`, toggle: Boolean(x.remote) })}
        ${row({ title: t('settings.agents.customCopy', { name: nameSpan(id) }), desc: esc(t(x.preset === 'booper' ? 'settings.agents.copyBooperHelp' : 'settings.agents.copyCustomHelp', { name: x.name })), action: `agent-copy-custom:${id}` })}
        ${row({ title: t('settings.agents.customRemove', { name: nameSpan(id) }), action: `agent-custom-remove:${id}`, cls: 'danger' })}
      </div>`;
    };
    return `
      <p class="settings-intro" data-i18n="settings.agents.help">${esc(t('settings.agents.help'))}</p>
      <div class="card">
        ${row({ title: t('settings.agents.default'), value: `<span data-agent-default>${esc(agentName(c, c.defaultAgent))}</span>`, action: 'agent-default' })}
        ${row({ title: t('settings.agents.shortcuts'), desc: t('settings.agents.shortcutsHelp'), action: 'agent-shortcuts' })}
        ${row({ title: t('settings.agents.permission'), desc: t('settings.agents.permissionHelp'), value: `<span data-agent-permission>${esc(labelOf(PERMISSION, c.permission))}</span>`, action: 'agent-permission' })}
      </div>
      ${head('clark')}
      <div class="card agent-card">
        ${statusRow('clark')}
        ${row({ title: t('settings.agents.enabled', { name: nameSpan('clark') }), action: 'agent-enabled:clark', toggle: c.clark.enabled !== false })}
        <div class="form agent-form">
          ${agentField('clark', 'name', t('settings.agents.name'), c.clark.name, 'Hermes')}
          ${agentField('clark', 'url', t('settings.agents.url'), c.clark.url, 'http://100.64.0.1:8642')}
          <div class="agent-key-box" data-agent-key-box="clark">${keyHtml('clark')}</div>
          ${agentField('clark', 'model', t('settings.agents.model'), c.clark.model, t('settings.agents.optional'))}
        </div>
        ${row({ title: t('settings.agents.remote', { name: nameSpan('clark') }), desc: `<span data-agent-address>${remoteDesc()}</span>`, action: 'agent-remote', toggle: Boolean(c.mcp && c.mcp.remote) })}
        ${row({ title: t('settings.agents.copySetup'), desc: t('settings.agents.copySetupHelp', { name: nameSpan('clark') }), action: 'agent-copy-setup' })}
      </div>
      ${head('claude')}
      <div class="card agent-card">
        ${statusRow('claude')}
        ${row({ title: t('settings.agents.enabled', { name: 'Claude' }), action: 'agent-enabled:claude', toggle: c.claude.enabled !== false })}
        <div class="form agent-form">
          ${agentField('claude', 'exe', t('settings.agents.exe'), c.claude.exe, exeHint('claude', 'claude.exe'))}
          ${agentField('claude', 'configDir', t('settings.agents.configDir'), c.claude.configDir, t('settings.agents.configDirHint'))}
          ${agentField('claude', 'model', t('settings.agents.model'), c.claude.model, t('settings.agents.modelHint', { example: 'opus' }))}
        </div>
        ${accessRow('claude')}
      </div>
      ${head('codex')}
      <div class="card agent-card">
        ${statusRow('codex')}
        ${row({ title: t('settings.agents.enabled', { name: 'Codex' }), action: 'agent-enabled:codex', toggle: c.codex.enabled !== false })}
        <div class="form agent-form">
          ${agentField('codex', 'exe', t('settings.agents.exe'), c.codex.exe, exeHint('codex', 'codex.exe'))}
          ${agentField('codex', 'model', t('settings.agents.model'), c.codex.model, t('settings.agents.optional'))}
        </div>
        ${accessRow('codex')}
      </div>
      ${head('openclaw')}
      <div class="card agent-card">
        ${statusRow('openclaw')}
        ${row({ title: t('settings.agents.enabled', { name: 'OpenClaw' }), action: 'agent-enabled:openclaw', toggle: c.openclaw.enabled !== false })}
        <div class="form agent-form">
          ${agentField('openclaw', 'url', t('settings.agents.gatewayUrl'), c.openclaw.url, 'http://127.0.0.1:18789')}
          <div class="agent-key-box" data-agent-key-box="openclaw">${keyHtml('openclaw')}</div>
          ${agentField('openclaw', 'agent', t('settings.agents.openclawAgent'), c.openclaw.agent, t('settings.agents.openclawAgentHint'))}
          ${agentField('openclaw', 'model', t('settings.agents.model'), c.openclaw.model, t('settings.agents.modelHint', { example: 'openai/gpt-5.4' }))}
        </div>
        ${row({ title: t('settings.agents.openclawRemote'), desc: `<span data-openclaw-address>${openclawRemoteDesc()}</span>`, action: 'agent-openclaw-remote', toggle: Boolean(c.openclaw.remote) })}
        ${row({ title: t('settings.agents.copyOpenClaw'), desc: esc(openclawCopyDesc()), action: 'agent-copy-openclaw' })}
      </div>
      ${(c.custom || []).map(customCard).join('')}
      <div class="card agent-card">
        ${row({ title: esc(t('settings.agents.customAdd')), desc: esc(t('settings.agents.customAddHelp')), action: 'agent-custom-add', lead: `<span class="add-ic">${icons.plus}</span>` })}
      </div>
      <div class="settings-group-title">${esc(t('settings.phone.title'))}</div>
      <div class="card agent-card" data-phone-card>${phoneRows()}</div>`;
  }

  // ----- shortcuts -----

  // The quick actions of the chat panel, as the user arranged them (actions.js shortcutList).
  const shortcuts = () => shortcutList(agentState && agentState.config.shortcuts);
  const shortcutRow = (s, i, list) => {
    const label = esc(s.label);
    const desc = [`/${esc(s.key)}`, esc(shortcutWhen(s)), !s.custom && (s.edited.label || s.edited.prompt) ? esc(t('settings.shortcuts.edited')) : ''].filter(Boolean).join(' · ');
    return `<div class="row static shortcut-row ${s.hidden ? 'is-hidden' : ''}" data-shortcut="${esc(s.key)}">
      <button class="shortcut-main" data-a="sc-edit:${esc(s.key)}" title="${esc(t('settings.shortcuts.editTitle'))}">
        <span class="shortcut-lead">${panelIcon(s.icon, 16).outerHTML}</span>
        <div class="text"><div class="title">${label}</div><div class="desc">${desc}</div></div>
      </button>
      <button class="icon-btn sm" data-a="sc-up:${esc(s.key)}" aria-label="${esc(t('settings.shortcuts.moveUp', { name: s.label }))}" title="${esc(t('settings.shortcuts.moveUp', { name: s.label }))}" ${i === 0 ? 'disabled' : ''}>${icons.up}</button>
      <button class="icon-btn sm" data-a="sc-down:${esc(s.key)}" aria-label="${esc(t('settings.shortcuts.moveDown', { name: s.label }))}" title="${esc(t('settings.shortcuts.moveDown', { name: s.label }))}" ${i === list.length - 1 ? 'disabled' : ''}>${icons.down}</button>
      <button class="shortcut-switch" data-a="sc-show:${esc(s.key)}" role="switch" aria-checked="${!s.hidden}" aria-label="${esc(t('settings.shortcuts.show', { name: s.label }))}" title="${esc(t('settings.shortcuts.show', { name: s.label }))}"><span class="switch ${s.hidden ? '' : 'on'}"></span></button>
    </div>`;
  };
  const shortcutRows = () => {
    const list = shortcuts();
    return `${list.map((s, i) => shortcutRow(s, i, list)).join('')}
      ${row({ title: t('settings.shortcuts.add'), action: 'sc-add', lead: `<span class="add-ic">${icons.plus}</span>`, cls: 'indent' })}`;
  };
  function shortcutsHtml() {
    return `
      <p class="settings-intro">${esc(t('settings.shortcuts.help'))}</p>
      <div class="card shortcut-card" data-shortcuts>${shortcutRows()}</div>
      <div class="settings-group-title"></div>
      <div class="card">${row({ title: t('settings.shortcuts.reset'), desc: t('settings.shortcuts.resetHelp'), action: 'sc-reset' })}</div>`;
  }
  // Saves the list and draws the rows again in place, so the page keeps its scroll; focus: the control to give the
  // focus back to, as a data-a value (a move button that is now disabled hands it to the other one).
  async function saveShortcuts(list, focus) {
    if (!(await updateAgents({ shortcuts: storedShortcuts(list) }))) return false;
    const card = page.querySelector('[data-shortcuts]');
    if (!card) return true;
    card.innerHTML = shortcutRows();
    if (focus) {
      let el = card.querySelector(`[data-a="${focus}"]`);
      if (el && el.disabled) el = card.querySelector(`[data-a="${focus.replace(/^sc-(up|down)/, (m, d) => `sc-${d === 'up' ? 'down' : 'up'}`)}"]`);
      el?.focus();
    }
    return true;
  }
  async function shortcutAction(action) {
    const [name, key] = action.split(':');
    const list = shortcuts();
    const i = list.findIndex((s) => s.key === key);
    const s = list[i];
    switch (name) {
      case 'sc-up':
      case 'sc-down': {
        const j = name === 'sc-up' ? i - 1 : i + 1;
        if (!s || j < 0 || j >= list.length) return;
        [list[i], list[j]] = [list[j], list[i]];
        return saveShortcuts(list, action);
      }
      case 'sc-show':
        if (!s) return;
        s.hidden = !s.hidden;
        return saveShortcuts(list, action);
      case 'sc-edit': {
        if (!s) return;
        const v = await shortcutDialog(s, list);
        if (!v) return;
        if (v === 'delete') list.splice(i, 1);
        else if (v === 'restore') s.edited = { label: '', prompt: '' };
        else if (s.custom) list[i] = { ...s, ...v, needsEmail: v.when === 'email', description: v.label };
        else s.edited = { label: v.label === s.defaultLabel ? '' : v.label, prompt: v.prompt === s.defaultPrompt ? '' : v.prompt };
        if (await saveShortcuts(list, v === 'delete' ? 'sc-add' : `sc-edit:${list[i] ? list[i].key : ''}`)) toast(t('settings.agents.saved'), 1400);
        return;
      }
      case 'sc-add': {
        if (list.length >= SHORTCUTS_MAX) return toast(t('settings.shortcuts.tooMany', { count: SHORTCUTS_MAX }), 5000);
        const v = await shortcutDialog(null, list);
        if (!v || typeof v !== 'object') return;
        list.push({ ...v, custom: true, hidden: false, needsEmail: v.when === 'email', description: v.label, action: 'shortcut' });
        if (await saveShortcuts(list, `sc-edit:${v.key}`)) toast(t('settings.shortcuts.added', { name: v.label }), 2400);
        return;
      }
      case 'sc-reset': {
        const ok = await confirmDialog(t('settings.shortcuts.resetTitle'), t('settings.shortcuts.resetConfirm'), t('settings.shortcuts.resetOk'));
        if (!ok) return;
        if (await updateAgents({ shortcuts: null })) {
          const card = page.querySelector('[data-shortcuts]');
          if (card) card.innerHTML = shortcutRows();
          toast(t('settings.shortcuts.resetDone'), 2400);
        }
        return;
      }
    }
  }

  // Saves a patch and reads the config back, so the page shows what main accepted.
  async function updateAgents(patch) {
    try {
      await api('agentUpdateConfig', patch);
      agentState.config = await api('agentConfig');
      return true;
    } catch (err) {
      toast(err.message, 5000);
      return false;
    }
  }
  const flip = (b, on) => {
    const sw = b.querySelector('.switch');
    sw.classList.toggle('on', on);
    sw.setAttribute('aria-checked', String(on));
  };

  async function testAgent(id) {
    showAgentStatus(id, { testing: true });
    try {
      const result = await api('agentTest', id);
      agentState.status = { ...(agentState.status || {}), [id]: result };
    } catch (err) {
      agentState.status = { ...(agentState.status || {}), [id]: { state: 'unknown', detail: err.message } };
    }
    showAgentStatus(id);
  }

  async function agentAction(action, b) {
    if (!agentState) return;
    const [name, id] = action.split(':');
    const c = agentState.config;
    switch (name) {
      case 'agent-shortcuts':
        return go('shortcuts');
      case 'agent-default': {
        const v = await choiceDialog(t('settings.agents.default'), allAgentIds(c).map((x) => ({ value: x, label: agentName(c, x), hint: productOf(c, x) })), c.defaultAgent);
        if (v && (await updateAgents({ defaultAgent: v }))) b.querySelector('[data-agent-default]').textContent = agentName(agentState.config, v);
        return;
      }
      case 'agent-permission': {
        const v = await choiceDialog(t('settings.agents.permission'), PERMISSION, c.permission, { stacked: true });
        if (v && (await updateAgents({ permission: v }))) b.querySelector('[data-agent-permission]').textContent = labelOf(PERMISSION, agentState.config.permission);
        return;
      }
      case 'agent-enabled':
        if (await updateAgents(agentPatch(id, { enabled: agentConf(c, id).enabled === false }))) {
          flip(b, agentConf(agentState.config, id).enabled !== false);
          if (isCustom(id)) showAddresses();
        }
        return;
      case 'agent-custom-add': {
        const preset = await choiceDialog(
          t('settings.agents.customAdd'),
          CUSTOM_PRESETS.map((x) => ({ value: x, label: t(`settings.agents.preset.${x}`), hint: t(`settings.agents.presetHint.${x}`) })),
          null,
          { stacked: true }
        );
        if (!preset) return;
        try {
          await api('agentAddCustom', preset);
        } catch (err) {
          return toast(err.message, 5000);
        }
        return render();
      }
      case 'agent-custom-remove': {
        const x = customOf(c, id);
        if (!x) return;
        if (!(await confirmDialog(t('settings.agents.customRemove', { name: x.name }), t('settings.agents.customRemoveConfirm', { name: x.name }), t('settings.agents.customRemoveOk'), true))) return;
        try {
          await api('agentRemoveCustom', id);
        } catch (err) {
          return toast(err.message, 5000);
        }
        toast(t('settings.agents.customRemoved', { name: x.name }), 4000);
        return render();
      }
      case 'agent-custom-remote':
        if (await updateAgents(agentPatch(id, { remote: !agentConf(c, id).remote }))) {
          flip(b, Boolean(agentConf(agentState.config, id).remote));
          showAddresses();
        }
        return;
      case 'agent-custom-history':
        if (await updateAgents(agentPatch(id, { history: agentConf(c, id).history === false }))) flip(b, agentConf(agentState.config, id).history !== false);
        return;
      case 'agent-copy-custom': {
        const x = agentConf(c, id);
        if (x.remote && !(c.mcp && c.mcp.address && c.mcp.remotePort)) return toast(t('settings.agents.openclawNoListener'), 6000);
        try {
          const copied = await api('agentCopyCustomSetup', id);
          toast(copied && copied.remote ? t('settings.agents.copiedCustomRemote', { name: x.name, url: copied.url }) : t('settings.agents.copiedCustom', { name: x.name }), 8000);
        } catch (err) {
          toast(err.message, 5000);
        }
        return;
      }
      case 'agent-access': {
        const v = await choiceDialog(t('settings.agents.access'), ACCESS, c[id].access);
        if (v && (await updateAgents({ [id]: { access: v } }))) b.querySelector('.value').textContent = labelOf(ACCESS, v);
        return;
      }
      case 'agent-remote':
        if (await updateAgents({ mcp: { remote: !(c.mcp && c.mcp.remote) } })) {
          flip(b, Boolean(agentState.config.mcp.remote));
          showAddresses();
        }
        return;
      case 'agent-phone':
        if (await updateAgents({ phone: { enabled: !(c.phone && c.phone.enabled) } })) {
          flip(b, Boolean(agentState.config.phone.enabled));
          showAddresses();
        }
        return;
      case 'agent-phone-pair':
        return pairPhone();
      case 'agent-phone-manage':
        return managePhone(id);
      case 'agent-openclaw-remote':
        if (await updateAgents({ openclaw: { remote: !c.openclaw.remote } })) {
          flip(b, Boolean(agentState.config.openclaw.remote));
          showAddresses();
        }
        return;
      case 'agent-test':
        return testAgent(id);
      case 'agent-copy-setup':
        try {
          await api('agentCopyHermesSetup');
          toast(t('settings.agents.copied', { name: agentName(c, 'clark') }), 6000);
        } catch (err) {
          toast(err.message, 5000);
        }
        return;
      case 'agent-copy-openclaw':
        if (!c.openclaw.hasKey) return toast(t('settings.agents.tokenFirst'), 5000);
        if (c.openclaw.remote && !(c.mcp && c.mcp.address && c.mcp.remotePort)) return toast(t('settings.agents.openclawNoListener'), 6000);
        try {
          const copied = await api('agentCopyOpenClawSetup');
          toast(copied && copied.remote ? t('settings.agents.copiedOpenClawRemote', { url: copied.url }) : t('settings.agents.copiedOpenClaw'), 8000);
        } catch (err) {
          toast(err.message, 5000);
        }
        return;
      case 'agent-key-replace':
      case 'agent-key-cancel':
        keyEditing[id] = name === 'agent-key-replace';
        page.querySelector(`[data-agent-key-box="${id}"]`).innerHTML = keyHtml(id);
        page.querySelector(`[data-agent-key="${id}"]`)?.focus();
        return;
      case 'agent-key-reveal': {
        const input = page.querySelector(`[data-agent-key="${id}"]`);
        const show = input.type === 'password';
        input.type = show ? 'text' : 'password';
        b.innerHTML = show ? icons.eyeOff : icons.eye;
        b.title = t(show ? keyText(id).hide : keyText(id).show);
        return;
      }
      case 'agent-key-save': {
        const input = page.querySelector(`[data-agent-key="${id}"]`);
        const value = input.value.trim();
        if (!value) return input.focus();
        try {
          await api('agentSetSecret', id, value);
          agentState.config = await api('agentConfig');
        } catch (err) {
          return toast(err.message, 5000);
        }
        keyEditing[id] = false;
        page.querySelector(`[data-agent-key-box="${id}"]`).innerHTML = keyHtml(id);
        showAddresses();
        toast(t(keyText(id).stored));
        return testAgent(id);
      }
    }
  }

  async function saveAgentField(input) {
    if (!agentState) return;
    const [id, key] = input.dataset.agentField.split('.');
    const value = input.value.trim();
    if (String(agentConf(agentState.config, id)[key] || '') === value) return;
    if (!(await updateAgents(agentPatch(id, { [key]: value })))) {
      input.value = agentConf(agentState.config, id)[key] || '';
      return;
    }
    input.value = agentConf(agentState.config, id)[key] || '';
    toast(t('settings.agents.saved'), 1400);
    // A custom agent's heading shows its kind beside a name of the user's own, so it is drawn again.
    if (isCustom(id) && key === 'name') return render();
    if ((id === 'clark' || isCustom(id)) && key === 'name') {
      page.querySelectorAll(`[data-agent-name="${id}"]`).forEach((el) => (el.textContent = agentName(agentState.config, id)));
      const def = page.querySelector('[data-agent-default]');
      if (def) def.textContent = agentName(agentState.config, agentState.config.defaultAgent);
    }
    if (key === 'url' || key === 'exe' || (id === 'openclaw' && key === 'agent')) testAgent(id);
  }

  page.addEventListener('change', (e) => {
    const input = e.target.closest && e.target.closest('[data-agent-field]');
    if (input) saveAgentField(input);
  });
  page.addEventListener('keydown', (e) => {
    if (e.key !== 'Enter') return;
    if (e.target.matches && e.target.matches('[data-agent-field]')) e.target.blur();
    if (e.target.matches && e.target.matches('[data-agent-key]')) page.querySelector(`[data-a="agent-key-save:${e.target.dataset.agentKey}"]`)?.click();
  });

  async function render() {
    const state = await api('state');
    ctx.S.data = state;
    const s = state.settings;
    setLanguage(s.language);
    localize();
    const view = stack[stack.length - 1];
    let title = t('settings.title');
    let body = '';
    // What a page needs from main, asked for at once so a redraw takes one round trip, not three.
    const [agents, updatesState] = await Promise.all([
      view === 'main' || view === 'agents' || view === 'shortcuts' ? loadAgents() : agentState,
      view === 'main' ? api('updatesState').catch(() => updates) : updates,
      view === 'main' || ACCOUNT_VIEWS.includes(view) ? account.prepare(view) : null,
      view === 'main' || view === 'rukoo-privacy' || CATEGORY_VIEWS.includes(view) ? categoriesPage.prepare() : null,
      view === 'main' || CALENDAR_VIEWS.includes(view) ? calendarPage.prepare(view, state) : null,
      view === 'main' || view === 'account' || view === 'signatures' || BOOKING_VIEWS.includes(view) ? bookingPage.prepare(view, state) : null,
      view === 'main' || MEETING_VIEWS.includes(view) ? meetingPage.prepare() : null,
      view === 'main' || VIDEO_VIEWS.includes(view) ? videoPage.prepare(view) : null,
      view === 'main' ? api('notificationState').then((n) => (notices.supported = n.supported)).catch(() => null) : null,
    ]);
    if (stack[stack.length - 1] !== view) return;
    if (view === 'main' || view === 'agents' || view === 'shortcuts') agentState = agents;
    updates = updatesState;

    if (view === 'main') {
      const accounts = state.accounts
        .map((a) => {
          const status = a.syncing
            ? t('mailbox.sync.syncing')
            : a.error
              ? `<span class="error">${esc(a.error)}</span>`
              : a.lastSync
                ? t('settings.account.lastSync', { date: numericDate(a.lastSync), time: hhmm(a.lastSync) })
                : t('mailbox.sync.never');
          // A button that opens the account; the default one has a badge, the others a button that makes them the
          // default. Two buttons side by side, because a button can't sit inside the row's own.
          const side = a.isDefault
            ? ''
            : `<button class="btn sm secondary" data-a="acc-make-default:${esc(a.id)}" aria-describedby="acc-email-${esc(a.id)}">${esc(t('settings.accounts.makeDefault'))}</button>`;
          return `<div class="row static indent acc-row">
            <button class="acc-main" data-a="account:${esc(a.id)}">
              ${accountAvatar(a)}
              <div class="text"><div class="title acc-title"><span class="acc-email" id="acc-email-${esc(a.id)}">${esc(a.email)}</span>${a.isDefault ? defaultBadge() : ''}</div>
              <div class="desc">${status}</div></div>
            </button>${side}
          </div>`;
        })
        .join('');
      // Calendar and the booking page get a group after General.
      const calendarGroup = `
        <div class="settings-group-title" data-i18n="settings.groups.calendar">${esc(t('settings.groups.calendar'))}</div>
        <div class="card">${calendarPage.rowHtml()}${bookingPage.rowHtml()}${videoPage.rowHtml()}</div>`;
      body = `
        <div class="settings-group-title" data-i18n="settings.groups.accounts">${esc(t('settings.groups.accounts'))}</div>
        <div class="card">${accounts}
          ${row({ title: t('settings.accounts.add'), action: 'add', cls: 'indent', lead: `<span class="add-ic">${icons.plus}</span>` })}
          ${row({ title: esc(t('settings.signatures.title')), desc: esc(t('settings.signatures.rowHelp')), action: 'signatures', cls: 'indent', lead: `<span class="sig-ic">${icons.edit}</span>` })}
        </div>
        <div class="settings-group-title" data-i18n="settings.groups.general">${esc(t('settings.groups.general'))}</div>
        <div class="card">
          ${row({ title: t('settings.general.language'), desc: t('settings.general.languageHelp'), value: languages.find((language) => language.code === s.language)?.name || 'English', action: 'language' })}
          ${row({ title: t('settings.general.folders'), desc: t('settings.general.foldersHelp'), action: 'folders' })}
          ${categoriesPage.rowHtml()}
          ${row({ title: t('settings.general.density'), value: labelOf(DENSITY, s.density), action: 'density' })}
          ${row({ title: t('settings.general.stackSenders'), desc: t('settings.general.stackSendersHelp'), action: 'toggle:stackSenders', toggle: s.stackSenders === true })}
          ${row({ title: t('settings.general.theme'), value: labelOf(THEMES, s.theme), action: 'theme' })}
          ${zoomRow(s.zoomLevel)}
          ${row({ title: t('settings.general.darkEmails'), desc: t('settings.general.darkEmailsHelp'), action: 'toggle:darkEmails', toggle: s.darkEmails })}
          ${row({ title: t('settings.swipe.title'), desc: t('settings.swipe.rowHelp'), action: 'swipe' })}
          ${row({ title: t('settings.keyboard.title'), desc: t('settings.keyboard.rowHelp'), value: esc(SCHEME_NAMES[s.keymap] || SCHEME_NAMES.rukoo), action: 'keyboard' })}
          ${row({ title: t('settings.general.fit'), desc: t('settings.general.fitHelp'), action: 'toggle:fitContent', toggle: s.fitContent })}
          ${row({ title: t('settings.general.logos'), desc: t('settings.general.logosHelp'), action: 'toggle:senderLogos', toggle: s.senderLogos !== false })}
          ${row({ title: t('settings.general.notifications'), desc: t('settings.general.notificationsHelp'), action: 'toggle:notifications', toggle: s.notifications })}
          ${row({ title: t('settings.general.newMailSound'), desc: t('settings.general.newMailSoundHelp'), action: 'toggle:newMailSound', toggle: s.newMailSound === true })}
          ${row({ title: t('settings.general.testNotification'), desc: esc(noticesDesc()), action: 'notify-test' })}
          ${row({ title: t('settings.general.badge'), value: labelOf(BADGES, s.badge), action: 'badge' })}
          ${row({ title: t('settings.general.sync'), value: labelOf(INTERVALS, Number(s.syncInterval)), action: 'interval' })}
          ${row({ title: t(window.mail.platform === 'darwin' ? 'settings.general.macMenuBarIcon' : 'settings.general.closeToTray'), desc: t(window.mail.platform === 'darwin' ? 'settings.general.macMenuBarIconHelp' : 'settings.general.closeToTrayHelp'), action: 'toggle:closeToTray', toggle: window.mail.platform === 'darwin' ? s.closeToTray || s.minimizeToTray : s.closeToTray })}
          ${row({ title: t(window.mail.platform === 'darwin' ? 'settings.general.macMinimize' : 'settings.general.minimizeToTray'), desc: t(window.mail.platform === 'darwin' ? 'settings.general.macMinimizeHelp' : 'settings.general.minimizeToTrayHelp'), action: 'toggle:minimizeToTray', toggle: s.minimizeToTray })}
          ${mailtoRow()}
          ${row({ title: t('settings.general.undoSend'), desc: t('settings.general.undoSendHelp'), value: labelOf(UNDO_SEND, Number(s.undoSend)), action: 'undo-send' })}
          ${row({ title: t('settings.general.spam'), desc: t('settings.general.spamHelp'), action: 'spam' })}
          ${row({ title: t('mailbox.folders.vip'), desc: t('settings.general.vipHelp'), action: 'vips' })}
        </div>
        ${calendarGroup}
        <div class="settings-group-title" data-i18n="settings.groups.meetings">${esc(t('settings.groups.meetings'))}</div>
        <div class="card">${meetingPage.rowHtml()}</div>
        <div class="settings-group-title" data-i18n="settings.groups.sync">${esc(t('settings.groups.sync'))}</div>
        <div class="card">${account.rowHtml()}</div>
        <div class="settings-group-title" data-i18n="settings.groups.agents">${esc(t('settings.groups.agents'))}</div>
        <div class="card">${row({
          title: agentState ? t('settings.agents.rowTitle', { name: esc(agentName(agentState.config, 'clark')) }) : t('settings.agents.title'),
          desc: agentState ? agentsRowDesc() : t('settings.agents.unavailable'),
          action: 'agents'
        })}</div>
        <div class="settings-group-title" data-i18n="settings.groups.about">${esc(t('settings.groups.about'))}</div>
        <div class="card">
          ${row({ title: t('whatsNew.settingsRow'), desc: t('whatsNew.settingsHelp'), action: 'whats-new' })}
          ${row({ title: t('settings.updates.title'), desc: t('settings.updates.help'), action: 'toggle:checkUpdates', toggle: s.checkUpdates !== false })}
          ${row({ title: t('settings.updates.channel'), value: esc(labelOf(CHANNELS, (updates && updates.channel) || s.updateChannel)), action: 'updates-channel' })}
          ${row({ title: t('settings.updates.checkNow'), value: esc(updatesValue()), action: 'updates-check' })}
          ${download() && download().status === 'ready' ? row({ title: t('settings.updates.restart'), desc: t('settings.updates.restartHelp'), action: 'updates-install' }) : ''}
          ${row({ title: t('settings.about.title'), desc: t('settings.about.help'), action: 'about' })}
        </div>`;
    }

    if (view === 'account') {
      const a = state.accounts.find((x) => x.id === params.id);
      if (!a) return back();
      title = a.email;
      // First on the page: whether new emails start from this account, or the way to make it so.
      const defaultRow = a.isDefault
        ? `<div class="row static" data-default-account><div class="text"><div class="title">${esc(t('settings.account.isDefault'))}</div><div class="desc">${esc(t('settings.account.makeDefaultHelp'))}</div></div></div>`
        : row({ title: t('settings.account.makeDefault'), desc: t('settings.account.makeDefaultHelp'), action: 'acc-default' });
      // The signature and the booking link under it come after the addresses that send them.
      const accountFooterRow = bookingPage.accountFooterRow(a, s);
      body = `
        <div class="settings-group-title" data-i18n="settings.groups.account">${esc(t('settings.groups.account'))}</div>
        <div class="card">
          ${defaultRow}
          ${row({ title: t('settings.account.name'), value: esc(a.name || ''), action: 'acc-name' })}
          ${row({ title: t('settings.account.color'), value: labelOf(COLORS, a.color), action: 'acc-color' })}
          ${row({ title: t('settings.account.from'), value: esc(a.defaultFrom), action: 'acc-from' })}
          ${row({ title: t('settings.account.identities'), desc: a.identities.length > 1 ? t('settings.aliases.count', { count: a.identities.length - 1 }) : t('settings.account.noAliases'), action: 'acc-aliases' })}
          ${accountSignatureRow(a, s)}
          ${accountFooterRow}
          ${row({ title: t('settings.account.sync'), desc: a.lastSync ? t('settings.account.lastSync', { date: numericDate(a.lastSync), time: hhmm(a.lastSync) }) : '', action: 'acc-sync' })}
          ${a.provider === 'google' && s && state.googleAvailable ? row({ title: a.auth === 'oauth2' ? t('settings.account.googleReauth') : t('settings.account.googleSwitch'), desc: a.auth === 'oauth2' ? t('settings.account.googleReauthHelp') : t('settings.account.googleSwitchHelp'), action: 'acc-google' }) : ''}
          ${a.type === 'imap' ? row({ title: t('settings.account.server'), desc: [a.imap.host, a.smtp.host, a.caldav && hostOf(a.caldav.url)].filter(Boolean).map(esc).join(' / '), action: 'acc-server' }) : ''}
        </div>
        <div class="settings-group-title"></div>
        <div class="card">${row({ title: t('settings.account.remove'), action: 'acc-remove', cls: 'danger' })}</div>`;
    }

    if (view === 'server') {
      const a = state.accounts.find((x) => x.id === params.id);
      if (!a) return back();
      title = t('settings.account.server');
      // Who emails the invitations when the user adds guests on this calendar server. Guests can only be added where
      // the server does; servers say so in their DAV header, and some get it wrong. Shown once a server is set.
      const invites = a.caldavInvites || '';
      const invitesHtml =
        a.auth !== 'oauth2' && a.caldav
          ? `<label data-i18n="settings.server.invites">${esc(t('settings.server.invites'))}<select name="caldavInvites">${[
              ['', 'settings.server.invitesAuto'],
              ['provider', 'settings.server.invitesProvider'],
              ['none', 'settings.server.invitesNone']
            ]
              .map(([value, key]) => `<option value="${value}" data-i18n="${key}"${value === invites ? ' selected' : ''}>${esc(t(key))}</option>`)
              .join('')}</select>
          <span class="field-help" data-i18n="settings.server.invitesHelp">${esc(t('settings.server.invitesHelp'))}</span></label>`
          : '';
      // What the server itself says, from the calendars Rukoo read last, goes with the first option.
      if (invitesHtml && !invites) {
        api('calendarList')
          .then((list) => {
            const acc = (list || []).find((x) => x.id === a.id);
            const said = acc && acc.calendars.length ? acc.calendars[0].invites : null;
            const option = page.querySelector('select[name="caldavInvites"] option[value=""]');
            if (!option || !said) return;
            const key = said === 'provider' ? 'settings.server.invitesAutoProvider' : 'settings.server.invitesAutoNone';
            option.dataset.i18n = key;
            option.textContent = t(key);
          })
          .catch(() => {});
      }
      body = `<div class="card"><form class="form" data-form="server">
        <label data-i18n="setup.fields.username">${esc(t('setup.fields.username'))}<input type="text" name="user" value="${esc(a.imap.user || a.email)}"/></label>
        ${a.auth === 'oauth2' ? '' : `<label data-i18n="setup.fields.password">${esc(t('setup.fields.password'))}<input type="password" name="password" data-i18n-placeholder="settings.server.passwordHint" placeholder="${esc(t('settings.server.passwordHint'))}"/></label>`}
        <div class="two"><label data-i18n="setup.fields.imap">${esc(t('setup.fields.imap'))}<input type="text" name="imapHost" value="${esc(a.imap.host)}"/></label>
          <label data-i18n="setup.fields.port">${esc(t('setup.fields.port'))}<input type="number" name="imapPort" value="${a.imap.port}"/></label>
          <label class="inline"><input type="checkbox" name="imapSecure" ${a.imap.secure ? 'checked' : ''}/> SSL/TLS</label></div>
        <div class="two"><label data-i18n="setup.fields.smtp">${esc(t('setup.fields.smtp'))}<input type="text" name="smtpHost" value="${esc(a.smtp.host)}"/></label>
          <label data-i18n="setup.fields.port">${esc(t('setup.fields.port'))}<input type="number" name="smtpPort" value="${a.smtp.port}"/></label>
          <label class="inline"><input type="checkbox" name="smtpSecure" ${a.smtp.secure ? 'checked' : ''}/> SSL/TLS</label></div>
        ${
          a.auth === 'oauth2'
            ? ''
            : `<label data-i18n="settings.server.caldav">${esc(t('settings.server.caldav'))}<input type="text" name="caldavUrl" value="${esc(a.caldav ? a.caldav.url : '')}" spellcheck="false" data-i18n-placeholder="settings.server.caldavPlaceholder" placeholder="${esc(t('settings.server.caldavPlaceholder'))}"/>
          <span class="field-help" data-i18n="settings.server.caldavHelp">${esc(t('settings.server.caldavHelp'))}</span></label>`
        }
        ${invitesHtml}
        <div class="error" data-error></div>
        <div><button class="btn" type="submit" data-i18n="common.actions.save">${esc(t('common.actions.save'))}</button></div>
      </form></div>`;
    }

    if (view === 'aliases') {
      const a = state.accounts.find((x) => x.id === params.id);
      if (!a) return back();
      title = t('settings.account.identities');
      // Under the addresses: the signature all of them send, since an account has one for all its addresses.
      const sentFrom = a.identities.length > 1 ? t('settings.aliases.signatureAll', { count: a.identities.length }) : t('settings.aliases.signatureOne');
      body = `
        <div class="settings-group-title" data-i18n="settings.aliases.sendAs">${esc(t('settings.aliases.sendAs'))}</div>
        <div class="card">
          ${a.identities
            .map((i) =>
              row({
                title: `<span class="acc-title"><span class="acc-email">${esc(i.address)}</span>${i.address === a.defaultFrom ? defaultBadge() : ''}</span>`,
                desc: i.primary ? t('settings.aliases.primary') : t('settings.aliases.manageHint'),
                action: i.primary ? `alias-default:${encodeURIComponent(i.address)}` : `alias:${encodeURIComponent(i.address)}`
              })
            )
            .join('')}
          ${row({ title: t('settings.aliases.add'), action: 'alias-add', lead: `<span class="add-ic">${icons.plus}</span>`, cls: 'indent' })}
          ${a.auth === 'oauth2' ? row({ title: t('settings.aliases.fetch'), desc: t('settings.aliases.fetchHelp'), action: 'alias-gmail' }) : ''}
        </div>
        <p class="settings-text" data-i18n="settings.aliases.help">${esc(t('settings.aliases.help'))}</p>
        <div class="settings-group-title" data-i18n="settings.general.signature">${esc(t('settings.general.signature'))}</div>
        <div class="card">${accountSignatureRow(a, s, esc(sentFrom))}</div>`;
    }

    if (view === 'signatures') {
      title = t('settings.signatures.title');
      // Each account with the addresses it sends from and what goes under their mail; an account without a signature
      // of its own shows the general one.
      const list = new Intl.ListFormat(getLocale(), { type: 'conjunction' });
      const accountRows = state.accounts
        .map((a) => {
          const aliases = a.identities.filter((i) => !i.primary && i.address !== a.email).map((i) => i.address);
          return row({
            title: `<span class="acc-title"><span class="acc-email">${esc(a.email)}</span>${a.isDefault ? defaultBadge() : ''}</span>`,
            desc: aliases.length ? esc(t('settings.signatures.aliases', { addresses: list.format(aliases) })) : '',
            ...accountSignatureFields(a, s, esc(t('settings.signatures.own'))),
            action: `acc-signature:${encodeURIComponent(a.id)}`,
            cls: 'indent sig-account',
            lead: accountAvatar(a)
          });
        })
        .join('');
      body = `
        <div class="card">
          ${row({ title: esc(t('settings.signatures.general')), desc: esc(t('settings.signatures.generalHelp')), ...generalSignatureFields(s), action: 'signature' })}
          ${bookingPage.footerRow(s)}
        </div>
        <div class="settings-group-title" data-i18n="settings.groups.accounts">${esc(t('settings.groups.accounts'))}</div>
        <div class="card">${accountRows}</div>`;
    }

    if (view === 'folders') {
      title = t('settings.general.folders');
      const hidden = new Set(s.hiddenViews || []);
      const folderRows = state.accounts.flatMap((a) =>
        a.folders.filter((f) => !f.role).map((f) => [`folder:${f.path}`, `${f.name} (${a.email})`])
      );
      body = `<div class="settings-group-title" data-i18n="settings.folders.show">${esc(t('settings.folders.show'))}</div><div class="card">${[...hideableViews(), ...folderRows]
        .map(([id, label]) => row({ title: esc(label), action: `hide:${id}`, toggle: !hidden.has(id) }))
        .join('')}</div>`;
    }

    if (view === 'spam' || view === 'vips') {
      const key = view === 'spam' ? 'spam' : 'vips';
      title = view === 'spam' ? t('settings.general.spam') : t('mailbox.folders.vip');
      const list = s[key] || [];
      body = `<div class="card">
        ${row({ title: t('common.actions.add'), action: `list-add:${key}`, lead: `<span class="add-ic">${icons.plus}</span>`, cls: 'indent' })}
        ${list.map((addr) => row({ title: esc(addr), desc: t('settings.addresses.removeHint'), action: `list-remove:${key}:${encodeURIComponent(addr)}` })).join('')}
      </div>
      ${list.length ? '' : `<p class="empty">${view === 'spam' ? t('settings.spam.empty') : t('settings.vip.empty')}</p>`}`;
    }

    if (view === 'agents') {
      title = t('settings.agents.title');
      keyEditing = {};
      body = agentState ? agentsHtml() : `<p class="empty">${esc(t('settings.agents.unavailable'))}</p>`;
    }

    if (view === 'keyboard') {
      title = t('settings.keyboard.title');
      const scheme = s.keymap || 'rukoo';
      const sample = (id) => ['archive', 'reply', 'star'].map((a) => `${t(`settings.keyboard.actions.${a}`)}: ${keyLabel(a, id)}`).join(' · ');
      const keys = (a) => bindings(a, scheme).map((b) => `<kbd>${esc(display(b))}</kbd>`).join('<span class="kbd-or"></span>');
      body = `
        <p class="settings-text">${esc(t('settings.keyboard.intro'))}</p>
        <div class="card">${SCHEMES.map((id) =>
          row({ title: esc(SCHEME_NAMES[id]), desc: esc(sample(id)), action: `keymap:${id}`, lead: `<span class="radio ${id === scheme ? 'on' : ''}" aria-hidden="true"></span>` })
        ).join('')}</div>
        <div class="settings-group-title">${esc(t('settings.keyboard.list', { scheme: SCHEME_NAMES[scheme] }))}</div>
        <div class="card">${KEY_ACTIONS.filter((a) => bindings(a, scheme).length)
          .map((a) => `<div class="row static"><div class="text"><div class="title">${esc(t(`settings.keyboard.actions.${a}`))}</div></div><span class="keys">${keys(a)}</span></div>`)
          .join('')}</div>
        <p class="settings-text">${esc(t('settings.keyboard.fixed'))}</p>`;
    }

    if (view === 'swipe') {
      title = t('settings.swipe.title');
      const quick = s.quickActions || [];
      body = `
        <div class="card">
          ${row({ title: t('settings.swipe.touch'), desc: t('settings.swipe.touchHelp'), action: 'toggle:swipeActions', toggle: s.swipeActions })}
          ${s.swipeActions ? row({ title: t('settings.swipe.right'), value: esc(t(`settings.actions.${s.swipeRight}`)), action: 'swipe-right' }) : ''}
          ${s.swipeActions ? row({ title: t('settings.swipe.left'), value: esc(t(`settings.actions.${s.swipeLeft}`)), action: 'swipe-left' }) : ''}
        </div>
        <div class="settings-group-title">${esc(t('settings.swipe.quick'))}</div>
        <div class="card">${QUICK.map((id) => row({ title: esc(t(`settings.actions.${id}`)), action: `quick:${id}`, toggle: quick.includes(id) })).join('')}</div>
        <p class="settings-text">${esc(t('settings.swipe.quickHelp'))}</p>`;
    }

    if (view === 'shortcuts') {
      title = t('settings.shortcuts.title');
      body = agentState ? shortcutsHtml() : `<p class="empty">${esc(t('settings.agents.unavailable'))}</p>`;
    }

    if (ACCOUNT_VIEWS.includes(view)) ({ title, body } = account.view(view));
    if (CATEGORY_VIEWS.includes(view)) ({ title, body } = categoriesPage.view(view));
    if (CALENDAR_VIEWS.includes(view)) ({ title, body } = calendarPage.view(view));
    if (BOOKING_VIEWS.includes(view)) ({ title, body } = bookingPage.view(view));
    if (MEETING_VIEWS.includes(view)) ({ title, body } = meetingPage.view(s));
    if (VIDEO_VIEWS.includes(view)) ({ title, body } = videoPage.view(view));

    // A redraw, such as the one after each update from main, keeps the keyboard on the control with the same action.
    const focused = page.contains(document.activeElement) ? document.activeElement.dataset.a : null;
    page.innerHTML = `
      <div class="page-bar"><button class="icon-btn" data-a="back" data-i18n-title="common.actions.back" title="${esc(t('common.actions.back'))}">${icons.back}</button><h1>${title}</h1></div>
      <div class="page-scroll"><div class="page-inner">${body}</div></div>`;
    if (focused) page.querySelector(`[data-a="${CSS.escape(focused)}"]`)?.focus({ preventScroll: true });
    if (view === 'agents' && agentState) allAgentIds(agentState.config).forEach((id) => showAgentStatus(id));
    if ((view === 'main' || view === 'agents') && agentState) refreshAgentStatus(view);
    if (view === 'main') showMailto();
    if (view === 'main' || ACCOUNT_VIEWS.includes(view)) account.fill(view);
    calendarPage.fill(view, page);
    bookingPage.fill(view, page);
    keepScroll(view);
  }

  page.addEventListener('submit', async (e) => {
    e.preventDefault();
    const form = e.target;
    if (String(form.dataset.form || '').startsWith('rukoo')) return account.submit(form);
    if (String(form.dataset.form || '').startsWith('booking')) return bookingPage.submit(form);
    const f = new FormData(form);
    const err = form.querySelector('[data-error]');
    const btn = form.querySelector('button[type=submit]');
    btn.disabled = true;
    btn.innerHTML = `<span class="spinner"></span>${esc(t('settings.server.checking'))}`;
    err.textContent = '';
    try {
      const acc = ctx.S.data.accounts.find((x) => x.id === params.id);
      const user = f.get('user').trim();
      const imap = { host: f.get('imapHost').trim(), port: Number(f.get('imapPort')), secure: f.get('imapSecure') === 'on', user };
      const smtp = { host: f.get('smtpHost').trim(), port: Number(f.get('smtpPort')), secure: f.get('smtpSecure') === 'on', user };
      const password = f.get('password') || undefined;
      // The mail servers are only checked again when something about them changed, so adding a calendar server
      // doesn't need the mail server to answer too.
      const same = (next, was) => Boolean(was) && next.host === was.host && next.port === Number(was.port) && next.secure === Boolean(was.secure) && next.user === (was.user || acc.email);
      const patch = password || !acc || !same(imap, acc.imap) || !same(smtp, acc.smtp) ? { imap, smtp, password } : {};
      if (f.has('caldavUrl')) patch.caldav = { url: f.get('caldavUrl').trim() };
      if (f.has('caldavInvites')) patch.caldavInvites = f.get('caldavInvites') || null;
      await api('updateAccount', params.id, patch);
      toast(t('settings.server.saved'));
      back();
    } catch (ex) {
      err.textContent = ex.message;
      btn.disabled = false;
      btn.textContent = t('common.actions.save');
    }
  });

  page.addEventListener('click', async (e) => {
    const b = e.target.closest('[data-a]');
    if (!b) return;
    const a = b.dataset.a;
    const s = ctx.S.data.settings;
    const set = async (patch) => {
      await api('updateSettings', patch);
      await render();
      ctx.refresh();
    };
    if (a === 'back') return back();
    if (a.startsWith('rukoo') && (await account.click(a))) return;
    if ((a === 'categories' || a.startsWith('cat-')) && (await categoriesPage.click(a))) return;
    if (a.startsWith('calset-') && (await calendarPage.click(a))) return;
    if (a.startsWith('booking-') && (await bookingPage.click(a))) return;
    if (a.startsWith('mtset-') && (await meetingPage.click(a))) return;
    if (a.startsWith('video-') && (await videoPage.click(a))) return;
    // Main answers with a 'zoom' event, which updates the row.
    if (a === 'zoom-reset') return api('setZoom', 100);
    if (a === 'notify-test') {
      if (notices.testing) return;
      notices.testing = true;
      const desc = b.querySelector('.desc');
      if (desc) desc.textContent = noticesDesc();
      notices.test = await api('testNotification').catch((err) => ({ result: 'failed', error: err.message }));
      notices.testing = false;
      return render();
    }
    if (a === 'mailto-use') {
      b.disabled = true;
      try {
        await api('useForMailto');
      } catch (err) {
        toast(err.message, 5000);
      }
      return showMailto();
    }
    if (a === 'agents') return go('agents');
    if (a.startsWith('agent-')) return agentAction(a, b);
    if (a.startsWith('sc-')) return shortcutAction(a);
    if (a === 'add') {
      close();
      return ctx.openSetup({ first: false });
    }
    if (a.startsWith('account:')) return go('account', { id: a.slice(8) });
    // An account's signature, from its page, its sender addresses or the Signatures page. Use general signature takes
    // its own away.
    if (a.startsWith('acc-signature:')) {
      const acc = ctx.S.data.accounts.find((x) => x.id === decodeURIComponent(a.slice(14)));
      if (!acc) return;
      const v = await editSignature({
        html: accountSignatureHtml(acc, s),
        title: t('settings.signature.accountTitle', { email: acc.email }),
        buttons: [{ get label() { return t('settings.signature.useGeneral'); }, value: { reset: true }, left: true }]
      });
      if (!v) return;
      try {
        await api('updateAccount', acc.id, v.reset ? { signature: null } : { signatureHtml: v.html });
      } catch (err) {
        return toast(err.message, 5000);
      }
      await render();
      return ctx.refresh();
    }
    if (a.startsWith('acc-make-default:')) {
      const id = a.slice(17);
      try {
        await api('updateAccount', id, { makeDefault: true });
      } catch (err) {
        toast(err.message, 5000);
      }
      await render();
      ctx.refresh();
      // The pressed button made way for the badge; keep the keyboard on that account's row.
      page.querySelector(`[data-a="account:${CSS.escape(id)}"]`)?.focus();
      return;
    }
    if (a.startsWith('toggle:')) {
      const key = a.slice(7);
      if (window.mail.platform === 'darwin' && key === 'closeToTray') {
        // Minimizing to the menu bar also needs the icon. Removing it turns that behavior off.
        return set(s.closeToTray || s.minimizeToTray
          ? { closeToTray: false, minimizeToTray: false }
          : { closeToTray: true });
      }
      await set({ [key]: !s[key] });
      if (key === 'fitContent' || key === 'darkEmails') ctx.redrawMail();
      return;
    }
    if (a.startsWith('keymap:')) return set({ keymap: a.slice(7) });
    if (a.startsWith('quick:')) {
      const id = a.slice(6);
      const quick = s.quickActions || [];
      if (quick.includes(id)) return set({ quickActions: quick.filter((x) => x !== id) });
      if (quick.length >= 4) return toast(t('settings.swipe.quickMax'));
      return set({ quickActions: QUICK.filter((x) => x === id || quick.includes(x)) });
    }
    if (a.startsWith('hide:')) {
      const id = a.slice(5);
      const hidden = new Set(s.hiddenViews || []);
      if (hidden.has(id)) hidden.delete(id);
      else hidden.add(id);
      return set({ hiddenViews: [...hidden] });
    }
    if (a.startsWith('alias-default:') || a.startsWith('alias:')) {
      const acc = ctx.S.data.accounts.find((x) => x.id === params.id);
      if (!acc) return;
      const address = decodeURIComponent(a.slice(a.indexOf(':') + 1));
      const setDefault = () => api('updateAccount', acc.id, { defaultFrom: address });
      let choice = 'default';
      if (a.startsWith('alias:')) {
        choice = await dialog({
          title: address,
          buttons: [
            { get label() { return t('common.actions.delete'); }, value: 'remove', danger: true },
            { get label() { return t('common.actions.cancel'); }, value: null },
            { get label() { return t('settings.aliases.makeDefault'); }, value: 'default' }
          ]
        });
      }
      try {
        if (choice === 'default') await setDefault();
        if (choice === 'remove') {
          const rest = acc.identities.filter((i) => !i.primary && i.address !== address).map((i) => ({ address: i.address, name: i.name }));
          await api('updateAccount', acc.id, { aliases: rest });
        }
      } catch (err) {
        toast(err.message, 5000);
      }
      await render();
      return ctx.refresh();
    }
    if (a.startsWith('list-add:')) {
      const key = a.slice(9);
      const v = await promptDialog(key === 'spam' ? t('settings.spam.add') : t('settings.vip.add'), '', { placeholder: t('setup.fields.emailExample') });
      const addr = String(v || '').trim().toLowerCase();
      if (!addr) return;
      if (!/^[^@\s]+@[^@\s]+$/.test(addr)) return toast(t('common.errors.invalidEmail'));
      return set({ [key]: [...new Set([...(s[key] || []), addr])] });
    }
    if (a.startsWith('list-remove:')) {
      const [, key, enc] = a.split(':');
      const addr = decodeURIComponent(enc);
      return set({ [key]: (s[key] || []).filter((x) => x !== addr) });
    }
    switch (a) {
      case 'language': {
        const value = await choiceDialog(t('settings.general.language'), languages.map((language) => ({ value: language.code, label: language.name })), s.language);
        if (value) await set({ language: value });
        return;
      }
      case 'folders':
      case 'spam':
      case 'vips':
      case 'keyboard':
      case 'swipe':
      case 'signatures':
        return go(a);
      case 'swipe-right':
      case 'swipe-left': {
        const key = a === 'swipe-right' ? 'swipeRight' : 'swipeLeft';
        const v = await choiceDialog(t(`settings.swipe.${a === 'swipe-right' ? 'right' : 'left'}`), SWIPE.map((id) => ({ value: id, label: t(`settings.actions.${id}`) })), s[key]);
        if (v) set({ [key]: v });
        return;
      }
      case 'whats-new':
        return ctx.openWhatsNew().then(() => render());
      case 'updates-check': {
        const state = await api('updatesCheck').catch(() => null);
        updates = state;
        await render();
        // A failed check says so, also when an earlier one found something.
        if (!state || state.failed) toast(t('settings.updates.failed'));
        else if (state.available) toast(updatesValue());
        else if (state.checkedAt) toast(t('settings.updates.upToDate'));
        else toast(t('settings.updates.failed'));
        return;
      }
      case 'updates-channel': {
        const current = (updates && updates.channel) || s.updateChannel || 'stable';
        const v = await choiceDialog(t('settings.updates.channel'), CHANNELS, current, { stacked: true });
        if (v && v !== current) set({ updateChannel: v });
        return;
      }
      case 'updates-install':
        return api('updatesInstall');
      case 'density': {
        const v = await choiceDialog(t('settings.general.density'), DENSITY, s.density);
        if (v) set({ density: v });
        return;
      }
      case 'theme': {
        const v = await choiceDialog(t('settings.general.theme'), THEMES, s.theme);
        if (v) set({ theme: v });
        return;
      }
      case 'badge': {
        const v = await choiceDialog(t('settings.general.badge'), BADGES, s.badge);
        if (v) set({ badge: v });
        return;
      }
      case 'interval': {
        const v = await choiceDialog(t('settings.general.sync'), INTERVALS, Number(s.syncInterval));
        if (v !== null && v !== undefined) set({ syncInterval: v });
        return;
      }
      case 'undo-send': {
        const v = await choiceDialog(t('settings.general.undoSend'), UNDO_SEND, Number(s.undoSend));
        if (v !== null && v !== undefined) set({ undoSend: v });
        return;
      }
      case 'signature': {
        const v = await editSignature({ html: signatureHtml(s.signature, s.signatureHtml), title: t('settings.signatures.general') });
        if (!v) return;
        try {
          await set({ signatureHtml: v.html });
        } catch (err) {
          toast(err.message, 5000);
        }
        return;
      }
      case 'about': {
        const info = await api('appInfo');
        return dialog({
          body: `${brandLogo('about-logo')}<p>${esc(t('settings.about.version', { version: info.version }))}</p><p data-i18n="settings.about.description">${esc(t('settings.about.description'))}<br><code style="user-select:text">${esc(info.dataDir)}</code></p>`
        });
      }
    }
    const acc = ctx.S.data.accounts.find((x) => x.id === params.id);
    if (!acc) return;
    const update = async (patch) => {
      try {
        await api('updateAccount', acc.id, patch);
        await render();
        ctx.refresh();
      } catch (err) {
        toast(err.message, 5000);
      }
    };
    switch (a) {
      case 'acc-name': {
        const v = await promptDialog(t('settings.account.name'), acc.name || '');
        if (v !== null) update({ name: v.trim() });
        return;
      }
      case 'acc-color': {
        const v = await choiceDialog(t('settings.account.color'), COLORS, acc.color);
        if (v) update({ color: v });
        return;
      }
      case 'acc-default':
        return update({ makeDefault: true });
      case 'acc-sync':
        toast(t('mailbox.sync.syncing'));
        try {
          await api('sync', acc.id);
          toast(t('mailbox.sync.done'));
        } catch (err) {
          toast(err.message, 5000);
        }
        return render();
      case 'acc-server':
        return go('server', { id: acc.id });
      case 'acc-aliases':
        return go('aliases', { id: acc.id });
      case 'acc-from': {
        const v = await choiceDialog(
          t('settings.account.from'),
          acc.identities.map((i) => ({ value: i.address, label: i.address })),
          acc.defaultFrom
        );
        if (v) update({ defaultFrom: v });
        return;
      }
      case 'alias-add': {
        const v = await promptDialog(t('settings.aliases.add'), '', { placeholder: t('setup.fields.emailExample') });
        if (!v || !v.trim()) return;
        const aliases = acc.identities.filter((i) => !i.primary).map((i) => ({ address: i.address, name: i.name }));
        update({ aliases: [...aliases, { address: v.trim(), name: acc.name }] });
        return;
      }
      case 'alias-gmail':
        try {
          const updated = await api('fetchGmailAliases', acc.id);
          toast(t('settings.aliases.fetched', { count: updated.identities.length - 1 }));
        } catch (err) {
          toast(err.message, 6000);
        }
        return render();
      case 'acc-google':
        toast(t('setup.google.waitToast'), 60000);
        try {
          await api('googleReauth', acc.id);
          toast(t('setup.google.signedIn'));
        } catch (err) {
          toast(err.message, 6000);
        }
        return render();
      case 'acc-remove': {
        const ok = await confirmDialog(t('settings.account.removeTitle'), t('settings.account.removeHelp', { email: acc.email }), t('common.actions.delete'), true);
        if (!ok) return;
        await api('removeAccount', acc.id);
        toast(t('settings.account.removed'));
        if (ctx.S.scope === acc.id) ctx.S.scope = 'all';
        ctx.closeReader();
        await ctx.refresh();
        if (!ctx.S.data.accounts.length) return close();
        return back();
      }
    }
  });

  render();
}
