// Settings > Booking page: the link where people, and their assistants, book time with the user. Signed out it says
// what the page needs; then one setup screen turns it on; after that the page, its meeting types, hours, bookings and
// requests. The booking service in main does the work (src/main/booking/, contracts f, g and h); this draws its state
// (bookingState, the 'booking' event) and sends the user's changes. Guests' text only ever goes through esc().
// The booking link under the signature (settings.bookingFooter) is set here too, beside the signature rows.
import { t, getLanguage, translate, getLocale } from './i18n.js';
import { icons } from './icons.js';
import { api, esc, toast, dialog, confirmDialog, choiceDialog, hhmm, KEEP_OPEN } from './ui.js';
import { duration, whenText, longDay, inlineDay, pcZone, zoneOptions } from './calendar-shared.js';
import { safeBookingUrl, bookingFooterHtml } from './composer.js';
import { signaturePreview } from './signature.js';
import { weekOrder, weekdayName, workHoursText } from './calendar-settings.js';
import { listText } from './video-settings.js';

export const BOOKING_VIEWS = ['booking', 'booking-setup', 'booking-type', 'booking-hours'];

const HANDLE = /^[a-z0-9](?:[a-z0-9]|-(?=[a-z0-9])){2,31}$/;
const SLUG = /^[a-z0-9](?:[a-z0-9]|-(?=[a-z0-9])){0,39}$/;
// The server's reserved handles and slugs (server/src/input.js), as in src/main/booking/rules.js, which the renderer
// can't load; test/booking-rules.test.js holds both copies to the server's.
const RESERVED_HANDLES = new Set(
  [
    'api assets v1 meet mcp llms admin help privacy rukoo www mail support account settings login signup book booking',
    'bookings cal calendar openapi well-known static abuse security status operator bvdm',
    'img media download downloads github releases styles app film video docs blog about pricing features terms legal',
    'press contact changelog android windows mac linux robots sitemap favicon index booker 404 500'
  ]
    .join(' ')
    .split(' ')
);
const RESERVED_SLUGS = new Set('api mcp booking bookings llms openapi ics assets manage prepare'.split(' '));
const MAX_TYPES = 10;
const MAX_RANGES = 6;
const FOOTER_MAX = 300;
const LENGTHS = [15, 20, 30, 45, 60, 90, 120, 180, 240];
const BUFFERS = [0, 5, 10, 15, 30, 45, 60];
const NOTICES = [0, 30, 60, 120, 240, 480, 720, 1440, 2880, 10080];
const HORIZONS = [7, 14, 30, 60, 90];
const INTERVALS = [5, 10, 15, 20, 30, 60];
const LIMITS = [1, 2, 3, 4, 5, 6, 8, 10, 15, 20];
const KINDS = ['googleMeet', 'phone', 'inPerson', 'link', 'none'];
const KIND_LABEL = {
  googleMeet: () => t('settings.booking.kindMeet'),
  phone: () => t('settings.booking.kindPhone'),
  inPerson: () => t('settings.booking.kindInPerson'),
  link: () => t('settings.booking.kindLink'),
  none: () => t('settings.booking.kindNone')
};
const KIND_VALUE = {
  phone: () => t('settings.booking.valuePhone'),
  inPerson: () => t('settings.booking.valueAddress'),
  link: () => t('settings.booking.valueLink')
};

const hostOf = (url) => {
  try {
    return new URL(url).host;
  } catch (_) {
    return '';
  }
};
const clone = (v) => JSON.parse(JSON.stringify(v));
// A name as a handle or slug: lowercase letters and digits, accents dropped, everything else one hyphen.
const slugify = (text, max) =>
  String(text || '')
    .normalize('NFD')
    .replace(/\p{M}/gu, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, max)
    .replace(/-+$/, '');
// "3 minutes ago", in the interface language. Under a minute is "just now": Intl's "this minute" is "binnen een
// minuut" in Dutch, which reads as still to come.
function ago(ms) {
  const rtf = new Intl.RelativeTimeFormat(getLocale(), { numeric: 'auto' });
  const minutes = Math.round((ms - Date.now()) / 60000);
  if (minutes === 0) return t('settings.booking.justNow');
  if (Math.abs(minutes) < 60) return rtf.format(minutes, 'minute');
  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 48) return rtf.format(hours, 'hour');
  return rtf.format(Math.round(hours / 24), 'day');
}
const noticeLength = (m) => (m === 0 ? t('settings.booking.noNotice') : m % 1440 === 0 ? t('settings.booking.days', { count: m / 1440 }) : duration(m * 60000));
const defaultType = (meet) => ({
  slug: '',
  title: '',
  duration: 30,
  durations: [30],
  description: '',
  location: { kind: meet ? 'googleMeet' : 'none', label: '', value: '', public: false },
  bufferBefore: 0,
  bufferAfter: 0,
  minNotice: 240,
  horizon: 30,
  slotInterval: null,
  dailyLimit: null,
  confirm: false,
  hidden: false
});
const options = (list, value, label) => list.map((v) => `<option value="${esc(v)}"${String(v) === String(value) ? ' selected' : ''}>${esc(label(v))}</option>`).join('');
// The lengths a guest can pick on a type, shortest first; a record from before there were several has only duration.
const lengthsOf = (type) => (Array.isArray(type.durations) && type.durations.length ? [...type.durations].sort((a, b) => a - b) : [type.duration]);
// "15 min, 30 min or 1 hr", in the interface language.
const lengthsText = (list) => {
  const parts = list.map((m) => duration(m * 60000));
  try {
    return new Intl.ListFormat(getLocale(), { type: 'disjunction' }).format(parts);
  } catch (_) {
    return parts.join(', ');
  }
};
// The title Rukoo gives a type, in the page's language (src/main/booking/rules.js): "30 minute meeting" with one
// length, "Meeting" with several. A title still equal to one Rukoo gave follows the lengths in the editor.
const madeTitle = (locale, lengths) => (lengths.length > 1 ? translate(locale, 'booking.defaults.typeTitleAny') : translate(locale, 'booking.defaults.typeTitle', { minutes: lengths[0] }));
const isMadeTitle = (locale, title, lengths) => title === madeTitle(locale, lengths) || lengths.some((m) => title === madeTitle(locale, [m]));
// The default length when the old one is no longer offered: the nearest one, the shorter of two as near.
const nearestLength = (lengths, old) => [...lengths].sort((a, b) => Math.abs(a - old) - Math.abs(b - old) || a - b)[0];
// The Default duration choices: the lengths picked under Available durations.
const defaultOptions = (lengths, value) => options(lengths, value, (m) => duration(m * 60000));

// pages: what settings.js needs. row: its row helper; go, back and rerender: its navigation; current(): the page on
// top; promptDialog: its text prompt.
export function bookingPages({ row, go, back, rerender, current, promptDialog }) {
  let state = null;
  let failed = null;
  let lists = null;
  let app = null;
  // The video services on this computer (videoState): the connected ones are locations a meeting type can have.
  let video = null;
  let lastView = null;
  // After Turn on: the card with Copy link, Preview page and Add to signature.
  let justOn = false;
  // What the setup screen holds, kept across redraws; the type and the hours being edited.
  let setup = null;
  let check = { handle: '', result: null, timer: null, seq: 0 };
  let edit = null;
  let hours = null;
  // What was drawn last: the settings page element, which screen of the booking page (for screen()), and what the
  // page, the overview's row and the footer rows showed of the state, so a 'booking' event that changes nothing shown
  // draws nothing.
  let host = null;
  let clock = null;
  let screen = null;
  let drawnPage = null;
  let drawnRow = null;
  let drawnFooter = null;

  const signedIn = () => Boolean(state && state.signedIn);
  const isOn = () => Boolean(state && state.status === 'on');
  // On, with the settings record on this computer; on another computer the record can still be on its way.
  const ready = () => Boolean(isOn() && state.settings);
  const url = () => (isOn() ? safeBookingUrl(state.url) || state.url : null);

  // ---------- calendars ----------

  const allCalendars = () => (lists || []).flatMap((a) => a.calendars.map((c) => ({ ...c, account: a, key: `${a.id}\n${c.id}`, portable: `${String(a.email).trim().toLowerCase()}\n${c.id}` })));
  // Where bookings can go (design 1.2): a writable Google calendar, a CalDAV one whose server sends invitations, or the
  // demo account's.
  const destinations = () => allCalendars().filter((c) => c.writable && (c.account.kind === 'google' || c.account.kind === 'demo' || c.invites === 'provider'));
  const byPortable = (p) => allCalendars().find((c) => c.portable === p) || null;
  const sendsInvites = (c) => c && c.account.kind !== 'demo' && c.invites === 'provider';
  const destinationNote = (c) => (!c ? '' : sendsInvites(c) ? t('settings.booking.invitesFrom', { address: c.account.email }) : t('settings.booking.noInvites'));
  const currentDestination = () => {
    const r = state && state.resolved && state.resolved.destination;
    if (r && r.key) return allCalendars().find((c) => c.key === r.key) || null;
    return state && state.settings && state.settings.destination ? byPortable(state.settings.destination) : null;
  };
  const meetAllowed = () => {
    const d = currentDestination();
    const r = state && state.resolved && state.resolved.destination;
    return Boolean((d && d.account.kind === 'google' && d.conference) || (r && r.conference));
  };

  // ---------- loading ----------

  // state: what settings.js just read from main. The booking page's own state is read again on every visit, the
  // server's too (refresh) when the page opens.
  async function prepare(view, s) {
    if (s) app = s;
    const entering = BOOKING_VIEWS.includes(view) && !BOOKING_VIEWS.includes(lastView);
    lastView = view;
    if (!BOOKING_VIEWS.includes(view) && view !== 'main') return;
    try {
      state = await api('bookingState', entering && view === 'booking' ? { refresh: true } : {});
      failed = null;
    } catch (err) {
      failed = err.message;
    }
    if (entering) justOn = false;
    if (BOOKING_VIEWS.includes(view) && signedIn()) [lists, video] = await Promise.all([api('calendarList').catch(() => lists), api('videoState').catch(() => video)]);
  }

  // ---------- video services ----------

  const videoServices = () => (video && Array.isArray(video.services) ? video.services : []);
  const videoName = (id) => (videoServices().find((s) => s.id === id) || { name: id }).name;
  // The services a type's location can be: on the computer that handles bookings, those connected here or needing
  // renewal; and the type's own whatever its state, so opening a type never changes its location.
  const videoOptions = (current) => videoServices().filter((s) => s.id === current || (state.thisDevice && (s.status === 'connected' || s.status === 'renew')));
  const videoLabel = (s) => (s.status === 'connected' ? s.name : s.status === 'renew' ? t('settings.booking.videoRenewOption', { service: s.name }) : t('settings.booking.videoOffOption', { service: s.name }));
  // A location select's value: 'video:zoom' for a video service, else the kind.
  const kindValue = (loc) => (loc.kind === 'video' ? `video:${loc.service}` : loc.kind);

  // What the page shows of a state. The publish line is left out: it is written in place, because a publish changes
  // only that line and a redraw would move the page under the user. Bookings that have ended drop out.
  const pageShown = (s) => {
    if (!s) return null;
    const { publishing, today, key, bookings, ...rest } = s;
    const now = Date.now();
    return JSON.stringify({ ...rest, bookings: (bookings || []).filter((b) => b.end > now && b.status === 'confirmed'), unchecked: publishing && publishing.unchecked });
  };
  // What the overview shows: the row's status and link, and the footer's language. The footer rows show the same:
  // the link while the page is on, in the page's language.
  const rowShown = (s) => JSON.stringify(s ? [s.status, s.url, s.settings && s.settings.locale] : null);

  function showPublish() {
    const line = host && host.isConnected ? host.querySelector('[data-booking-publish]') : null;
    if (line) line.innerHTML = publishLine();
  }

  function onState(next) {
    if (!next || typeof next !== 'object') return;
    const was = state && state.status;
    state = next;
    const view = current();
    // The setup screen and the editors keep what is typed; the page, the overview and the pages with the footer rows
    // (Signatures and an account's page) follow what they show.
    if (view === 'main') {
      if (rowShown(next) !== drawnRow) rerender();
    } else if (view === 'signatures' || view === 'account') {
      if (rowShown(next) !== drawnFooter) rerender();
    } else if (view === 'booking') {
      if (isOn() && drawnPage !== null && pageShown(next) === drawnPage) showPublish();
      else if (isOn() || was !== next.status) rerender();
    }
  }

  // ---------- the overview row and the footer rows ----------

  function rowHtml() {
    drawnRow = rowShown(state);
    const short = isOn() ? String(state.url || '').replace(/^https?:\/\//, '') : '';
    const desc = isOn() ? t('settings.booking.rowOn', { url: short }) : state && state.status === 'signed_out' ? t('settings.booking.rowSignedOut') : t('settings.booking.rowOff');
    return row({ title: esc(t('settings.booking.title')), desc: esc(desc), action: 'booking-open' });
  }

  const footerData = () => ({ ...(app || {}), bookingUrl: (app && app.bookingUrl) || url(), bookingLocale: (app && app.bookingLocale) || (state && state.settings && state.settings.locale) || null });
  const defaultFooterText = () => translate(footerData().bookingLocale || getLanguage(), 'composer.booking.footerDefault', { url: '{url}' });

  // Beside the general signature: the switch, the preview while the page is on, and the text.
  function footerRow(s) {
    drawnFooter = rowShown(state);
    const live = safeBookingUrl(footerData().bookingUrl);
    const preview = live && s.bookingFooter ? bookingFooterHtml(null, { ...footerData(), settings: s }) : '';
    let out = row({
      title: esc(t('settings.footer.title')),
      desc: esc(live ? t('settings.footer.help') : t('settings.footer.needsPage')),
      preview: preview ? signaturePreview(preview) : '',
      action: 'booking-footer',
      toggle: Boolean(s.bookingFooter)
    });
    if (s.bookingFooter) out += row({ title: esc(t('settings.footer.text')), value: esc(s.bookingFooterText || t('settings.footer.textDefault')), action: 'booking-footer-text' });
    return out;
  }

  // Beside an account's signature: following the general setting, or on or off for this account.
  function accountFooterRow(a, s) {
    drawnFooter = rowShown(state);
    const own = typeof a.bookingFooter === 'boolean';
    const general = s.bookingFooter ? t('settings.footer.on') : t('settings.footer.off');
    const value = own ? (a.bookingFooter ? (a.bookingFooterText ? t('settings.footer.onOwnText') : t('settings.footer.on')) : t('settings.footer.off')) : t('settings.footer.accountGeneral', { state: general });
    const live = safeBookingUrl(footerData().bookingUrl);
    const html = live ? bookingFooterHtml(a, { ...footerData(), settings: s }) : '';
    return row({ title: esc(t('settings.footer.title')), value: esc(value), preview: html ? signaturePreview(html) : '', action: `booking-acc-footer:${encodeURIComponent(a.id)}` });
  }

  // The footer's text: the default in the page's language, or the user's own with {url} where the link goes.
  function footerTextDialog(text) {
    return dialog({
      title: t('settings.footer.text'),
      wide: true,
      buttons: [
        { get label() { return t('settings.footer.useDefault'); }, value: 'default', left: true },
        { get label() { return t('common.actions.cancel'); }, value: null },
        {
          get label() { return t('common.actions.save'); },
          primary: true,
          value: (scrim) => {
            const v = scrim.querySelector('textarea').value.trim();
            const error = scrim.querySelector('[data-error]');
            if (!v.includes('{url}')) return (error.textContent = t('settings.footer.needUrl', { url: '{url}' })), KEEP_OPEN;
            if (v.length > FOOTER_MAX) return (error.textContent = t('settings.footer.tooLong', { count: FOOTER_MAX })), KEEP_OPEN;
            return v === defaultFooterText() ? 'default' : v;
          }
        }
      ],
      render(body) {
        body.innerHTML = `<div class="form" style="padding:0">
          <label>${esc(t('settings.footer.text'))}<textarea maxlength="${FOOTER_MAX + 20}" rows="3" spellcheck="true">${esc(text || defaultFooterText())}</textarea>
          <span class="field-help">${esc(t('settings.footer.textHelp', { url: '{url}' }))}</span></label>
          <div class="error" data-error></div></div>`;
      }
    });
  }

  // ---------- states before the page ----------

  function signedOutHtml() {
    const server = state.serverName || hostOf(state.server);
    return `<p class="settings-text">${esc(t('settings.booking.signedOutIntro'))}</p>
      ${state.invite ? `<p class="settings-text">${esc(t('settings.booking.inviteNote', { server }))}</p>` : ''}
      <div class="settings-group-title"></div>
      <div class="card">
        ${row({ title: esc(t('settings.booking.createAccount')), action: 'booking-account:rukoo-signup' })}
        ${row({ title: esc(t('settings.booking.signIn')), action: 'booking-account:rukoo-signin' })}
      </div>
      <p class="settings-text">${esc(t('settings.booking.privacyNote'))}</p>`;
  }

  function unsupportedHtml() {
    return `<p class="settings-text">${esc(t('settings.booking.unsupported', { server: state.serverName || hostOf(state.server) }))}</p>
      <div class="card">${row({ title: esc(t('settings.rukoo.title')), action: 'booking-account:rukoo' })}</div>`;
  }

  function defaultHandle() {
    const accounts = (app && app.accounts) || [];
    const acc = accounts.find((a) => a.isDefault) || accounts[0];
    if (!acc) return '';
    const fromName = slugify(acc.name, 32);
    if (fromName.length >= 3 && !RESERVED_HANDLES.has(fromName)) return fromName;
    const fromAddress = slugify(String(acc.email).split('@')[0], 32);
    return fromAddress.length >= 3 && !RESERVED_HANDLES.has(fromAddress) ? fromAddress : '';
  }

  function setupValues() {
    if (setup) return setup;
    const s = (state && state.settings) || {};
    const accounts = (app && app.accounts) || [];
    const acc = accounts.find((a) => a.isDefault) || accounts[0];
    const dests = destinations();
    const stored = currentDestination();
    const primary = acc && dests.find((c) => c.account.id === acc.id && c.primary);
    setup = {
      handle: s.handle || (state && state.handle) || defaultHandle(),
      name: s.name || (acc && (acc.name || String(acc.email).split('@')[0])) || '',
      timeZone: s.timeZone || pcZone(),
      // The stored destination, when bookings can still go there; each list is built anew, so keys are compared.
      destination: (stored && dests.some((c) => c.key === stored.key) && stored.key) || (primary && primary.key) || (dests[0] && dests[0].key) || '',
      calendars: Object.fromEntries(allCalendars().map((c) => [c.key, c.availability === true]))
    };
    return setup;
  }

  // The live answer for the handle field, before and after asking main.
  function handleStatus(handle) {
    if (!handle) return { text: t('settings.booking.handleHelp'), tone: '' };
    if (!HANDLE.test(handle)) return { text: t('settings.booking.handleHelp'), tone: 'error' };
    if (RESERVED_HANDLES.has(handle)) return { text: t('settings.booking.handleReserved'), tone: 'error' };
    const r = check.handle === handle ? check.result : null;
    if (!r) return { text: t('settings.booking.handleChecking'), tone: '' };
    if (r.error) return { text: r.error, tone: 'error' };
    if (r.available || r.reason === 'yours') return { text: r.reason === 'yours' ? t('settings.booking.handleYours') : t('settings.booking.handleAvailable'), tone: 'ok' };
    if (r.reason === 'taken') return { text: t('settings.booking.handleTaken'), tone: 'error' };
    if (r.reason === 'reserved') return { text: t('settings.booking.handleReserved'), tone: 'error' };
    return { text: t('settings.booking.handleHelp'), tone: 'error' };
  }

  function setupHtml() {
    const v = setupValues();
    const dests = destinations();
    // The page's address before its handle: from the page's own address, else as the server reports it (older servers
    // don't, and keep pages under /meet).
    const base = state.url ? state.url.replace(/[^/]*$/, '') : state.base ? `${state.base}/` : `${hostOf(state.server)}/meet/`;
    const status = handleStatus(v.handle);
    const calendarsHtml = (lists || [])
      .filter((a) => a.calendars.length)
      .map(
        (a) => `<div class="booking-cal-account">${esc(a.email)}</div>${a.calendars
          .map((c) => {
            const k = `${a.id}\n${c.id}`;
            const note = !c.owned && !c.primary ? t('calendar.sidebar.availabilityShared') : '';
            return `<label class="inline booking-cal"><input type="checkbox" name="cal" value="${esc(k)}" ${v.calendars[k] ? 'checked' : ''}/><span class="booking-cal-name">${esc(c.name)}${note ? `<span class="booking-cal-note">${esc(note)}</span>` : ''}</span></label>`;
          })
          .join('')}`
      )
      .join('');
    const groups = [...new Set(dests.map((c) => c.account.email))];
    const destOptions = groups
      .map((email) => `<optgroup label="${esc(email)}">${dests.filter((c) => c.account.email === email).map((c) => `<option value="${esc(c.key)}"${c.key === v.destination ? ' selected' : ''}>${esc(c.name)}</option>`).join('')}</optgroup>`)
      .join('');
    const dest = dests.find((c) => c.key === v.destination);
    const calendar = (app && app.settings && app.settings.calendar) || null;
    return `
      <p class="settings-text">${esc(state.settings ? t('settings.booking.offIntro') : t('settings.booking.setupIntro'))}</p>
      <div class="card"><form class="form booking-setup" data-form="booking-setup" novalidate>
        <label>${esc(t('settings.booking.handle'))}
          <span class="booking-handle"><span class="booking-handle-base" aria-hidden="true"><span dir="ltr">${esc(base.replace(/^https?:\/\//, ''))}</span></span><input type="text" name="handle" value="${esc(v.handle)}" maxlength="32" spellcheck="false" autocomplete="off" aria-describedby="booking-handle-status"/></span>
          <span class="field-help booking-status ${status.tone}" id="booking-handle-status" data-handle-status aria-live="polite">${esc(status.text)}</span></label>
        <label>${esc(t('settings.booking.name'))}<input type="text" name="name" value="${esc(v.name)}" maxlength="100" autocomplete="off"/></label>
        <label>${esc(t('settings.booking.zone'))}<select name="timeZone">${zoneOptions(v.timeZone)}</select></label>
        <div class="booking-field"><span class="booking-field-label">${esc(t('settings.booking.hours'))}</span>
          <span class="booking-field-value">${esc(t('settings.booking.hoursWorking', { hours: workHoursText(calendar) }))}</span>
          <span class="field-help">${esc(t('settings.booking.hoursSetupHelp'))}</span></div>
        <fieldset class="booking-cals"><legend>${esc(t('settings.booking.calendars'))}</legend>
          <span class="field-help">${esc(t('settings.booking.calendarsHelp'))}</span>
          ${calendarsHtml || `<span class="field-help">${esc(t('settings.calendar.noCalendars'))}</span>`}</fieldset>
        ${
          dests.length
            ? `<label>${esc(t('settings.booking.destination'))}<select name="destination">${destOptions}</select>
          <span class="field-help" data-destination-note>${esc(destinationNote(dest))}</span></label>`
            : `<p class="error">${esc(t('settings.booking.noDestination'))}</p>`
        }
        <p class="field-help">${esc(t('settings.booking.privacyNote'))}</p>
        <div class="error" data-error role="alert"></div>
        <div><button class="btn primary" type="submit" ${dests.length ? '' : 'disabled'}>${esc(t('settings.booking.turnOn'))}</button></div>
      </form></div>`;
  }

  // Asks main whether the handle can be had, a moment after the last key.
  function checkHandle(handle, page) {
    clearTimeout(check.timer);
    const show = () => {
      const el = page.querySelector('[data-handle-status]');
      if (!el) return;
      const s = handleStatus(handle);
      el.textContent = s.text;
      el.className = `field-help booking-status ${s.tone}`;
    };
    show();
    if (!HANDLE.test(handle) || RESERVED_HANDLES.has(handle) || (check.handle === handle && check.result)) return;
    check.timer = setTimeout(async () => {
      const seq = ++check.seq;
      let result;
      try {
        result = await api('bookingCheckHandle', handle);
      } catch (err) {
        result = { error: err.message };
      }
      if (seq !== check.seq) return;
      check = { ...check, handle, result };
      show();
    }, 300);
  }

  // ---------- the page ----------

  const kindName = (loc) => (loc && loc.label ? loc.label : loc && loc.kind === 'video' ? videoName(loc.service) : loc && loc.kind !== 'none' ? KIND_LABEL[loc.kind]() : '');
  // When guests can book a type, as far as it differs from a new type's defaults: "15 mins free time after",
  // "1 day notice". Equal free time on both sides is one phrase.
  const whenDesc = (type) => {
    const d = defaultType(false);
    const time = (m) => duration(m * 60000);
    const out = [];
    if (type.bufferBefore && type.bufferBefore === type.bufferAfter) out.push(t('settings.booking.rowBufferBoth', { time: time(type.bufferBefore) }));
    else {
      if (type.bufferBefore) out.push(t('settings.booking.rowBufferBefore', { time: time(type.bufferBefore) }));
      if (type.bufferAfter) out.push(t('settings.booking.rowBufferAfter', { time: time(type.bufferAfter) }));
    }
    if (type.minNotice !== d.minNotice) out.push(type.minNotice ? t('settings.booking.rowNotice', { time: noticeLength(type.minNotice) }) : t('settings.booking.rowNoNotice'));
    if (type.horizon !== d.horizon) out.push(t('settings.booking.rowHorizon', { count: type.horizon }));
    if (type.slotInterval != null && type.slotInterval !== d.slotInterval) out.push(t('settings.booking.rowInterval', { time: time(type.slotInterval) }));
    if (type.dailyLimit != null && type.dailyLimit !== d.dailyLimit) out.push(t('settings.booking.rowLimit', { count: type.dailyLimit }));
    return out;
  };
  // A row that wraps breaks after a dot, not before it.
  const typeDesc = (type) =>
    [lengthsText(lengthsOf(type)), kindName(type.location), ...whenDesc(type), type.confirm ? t('settings.booking.typeConfirm') : '', type.hidden ? t('settings.booking.typeHidden') : ''].filter(Boolean).join(' · ');
  const hoursValue = () => {
    const s = state.settings || {};
    const calendar = app && app.settings && app.settings.calendar;
    const base = s.hours && s.hours.custom ? t('settings.booking.hoursCustom') : t('settings.booking.hoursWorking', { hours: workHoursText(calendar) });
    const n = Object.keys(s.overrides || {}).length;
    return n ? `${base} · ${t('settings.booking.overridesCount', { count: n })}` : base;
  };

  function doneHtml() {
    const s = (app && app.settings) || {};
    const added = s.bookingFooter;
    return `<div class="card booking-done" role="status">
        <div class="booking-done-text"><div class="booking-done-title">${icons.check}<span>${esc(t('settings.booking.doneTitle'))}</span></div>
          <div class="booking-url">${esc(state.url)}</div></div>
        <div class="booking-done-actions">
          <button class="btn sm secondary" data-a="booking-copy">${icons.link}${esc(t('settings.booking.copyLink'))}</button>
          <button class="btn sm secondary" data-a="booking-preview">${icons.popOut}${esc(t('settings.booking.preview'))}</button>
          ${added ? `<span class="booking-added">${icons.check}${esc(t('settings.booking.inSignature'))}</span>` : `<button class="btn sm primary" data-a="booking-signature">${esc(t('settings.booking.addToSignature'))}</button>`}
        </div>
      </div>`;
  }

  function publishLine() {
    const p = state.publishing || {};
    if (!state.thisDevice) return '';
    if (p.lastError) return `<span class="error">${esc(p.lastError)}</span>`;
    const text = p.lastAt ? t('settings.booking.lastUpdated', { ago: ago(p.lastAt) }) : t('settings.booking.neverUpdated');
    return p.stale ? `<span class="booking-warn">${esc(text)}</span> · ${esc(t('settings.booking.staleHelp'))}` : esc(text);
  }

  function itemHtml(b, request) {
    const name = b.guest ? b.guest.name : t('settings.booking.guestUnknown');
    const bits = [whenText({ start: b.start, end: b.end, allDay: false })];
    if (b.guest && b.guest.via) bits.push(t('settings.booking.via', { via: b.guest.via }));
    if (b.customLength) bits.push(t('settings.booking.customLength'));
    if (request && b.expiresAt) bits.push(t('settings.booking.expires', { time: `${inlineDay(b.expiresAt)}, ${hhmm(b.expiresAt)}` }));
    const notes = b.guest && b.guest.notes ? `<div class="desc booking-notes">${esc(b.guest.notes)}</div>` : '';
    const id = esc(b.id);
    const actions = !state.thisDevice
      ? ''
      : request
        ? `<button class="btn sm" data-a="booking-decline:${id}">${esc(t('settings.booking.decline'))}</button><button class="btn sm secondary" data-a="booking-accept:${id}">${esc(t('settings.booking.accept'))}</button>`
        : `<button class="btn sm" data-a="booking-cancel:${id}">${esc(t('settings.booking.cancel'))}</button>`;
    return `<div class="row static booking-item">
        <div class="text"><div class="title"><span class="booking-guest">${esc(name)}</span> <span class="booking-type-name">· ${esc(b.typeTitle || b.type)}</span></div>
          <div class="desc">${esc(bits.join(' · '))}</div>${notes}</div>
        <div class="booking-item-actions">${actions}</div>
      </div>`;
  }

  function noticesHtml() {
    const list = state.notices || [];
    if (!list.length) return '';
    // Buttons name their notice and action, not their place in the list, which can change before a click is handled.
    return `<div class="card booking-notices">${list
      .map((n) => {
        const id = encodeURIComponent(n.id);
        const actions = (n.actions || [])
          .map((a, j) =>
            a.action === 'open' ? '' : `<button class="btn sm ${j === n.actions.length - 1 && a.action !== 'dismiss' ? 'secondary' : ''}" data-a="booking-notice:${esc(id)}:${esc(encodeURIComponent(a.action))}">${esc(a.label)}</button>`
          )
          .join('');
        return `<div class="row static booking-notice ${n.tone === 'error' ? 'error' : n.tone === 'warn' ? 'warn' : ''}">
          <span class="booking-notice-ic" aria-hidden="true">${n.tone === 'info' ? icons.bell : icons.info}</span>
          <div class="text"><div class="title">${esc(n.title)}</div><div class="desc">${esc(n.text)}</div></div>
          <div class="booking-item-actions">${actions}</div></div>`;
      })
      .join('')}</div>`;
  }

  // The page while it is on. Without its settings record (another computer's page, not pulled yet) it shows the link,
  // notices and bookings, and says the rest is on its way.
  function pageHtml() {
    const device = state.device;
    const unchecked = ((state.publishing && state.publishing.unchecked) || [])
      .map((u) => `<div class="row static"><div class="text"><div class="desc booking-warn">${esc(t('settings.booking.unchecked', { email: u.email, time: `${inlineDay(u.since)}, ${hhmm(u.since)}` }))}</div></div></div>`)
      .join('');
    // Use this computer moves the meetings too: the services the types use that aren't connected here.
    const missing = ((state.video && state.video.missing) || []).map((m) => m.name);
    const otherDevice =
      !state.thisDevice && device
        ? `<div class="row static booking-device">
            <div class="text"><div class="title">${esc(device.name ? t('settings.booking.otherDevice', { device: device.name }) : t('settings.booking.otherDeviceGone'))}</div>
              <div class="desc">${esc(t('settings.booking.otherDeviceHelp'))}</div>
              ${missing.length ? `<div class="desc booking-warn" data-video-missing>${esc(t('settings.booking.otherDeviceVideo', { services: listText(missing) }))}</div>` : ''}</div>
            <button class="btn sm secondary" data-a="booking-takeover">${esc(t('settings.booking.takeover'))}</button></div>`
        : '';
    const requests = state.requests || [];
    const bookings = (state.bookings || []).filter((b) => b.end > Date.now() && b.status === 'confirmed');
    return `
      ${justOn ? doneHtml() : ''}
      <div class="settings-group-title">${esc(t('settings.booking.yourLink'))}</div>
      <div class="card">
        <div class="row static booking-link-row">
          <div class="text"><div class="title booking-url">${esc(state.url || '')}</div><div class="desc" data-booking-publish>${publishLine()}</div></div>
          <button class="btn sm secondary" data-a="booking-copy" title="${esc(t('settings.booking.copyLink'))}">${icons.link}<span>${esc(t('settings.booking.copyLink'))}</span></button>
          <button class="btn sm secondary" data-a="booking-preview" title="${esc(t('settings.booking.preview'))}">${icons.popOut}<span>${esc(t('settings.booking.preview'))}</span></button>
        </div>
        ${otherDevice}
        ${unchecked}
        ${row({ title: esc(t('settings.booking.taking')), desc: esc(t('settings.booking.takingHelp')), action: 'booking-off', toggle: true })}
        ${row({ title: esc(t('settings.booking.syncNow')), desc: state.busy ? esc(t('settings.booking.syncing')) : '', action: 'booking-sync' })}
      </div>
      ${noticesHtml()}
      ${
        requests.length
          ? `<div class="settings-group-title">${esc(t('settings.booking.requests'))}</div><div class="card booking-list">${requests.map((b) => itemHtml(b, true)).join('')}</div>`
          : ''
      }
      <div class="settings-group-title">${esc(t('settings.booking.bookings'))}</div>
      ${bookings.length ? `<div class="card booking-list">${bookings.map((b) => itemHtml(b, false)).join('')}</div>` : `<p class="settings-text booking-empty">${esc(t('settings.booking.noBookings'))}</p>`}
      ${state.settings ? settingsHtml() : `<p class="settings-text booking-missing" role="status">${esc(t('settings.booking.settingsMissing'))}</p>`}`;
  }

  // The page's own settings: meeting types, hours, where bookings go, and what the page says.
  function settingsHtml() {
    const s = state.settings;
    const types = s.types || [];
    const dest = currentDestination();
    const r = state.resolved || {};
    const destName = dest ? `${dest.name} · ${dest.account.email}` : r.destination ? `${r.destination.name} · ${r.destination.email}` : t('settings.booking.destinationGone');
    const counted = (r.availability || []).filter((c) => c.found).length;
    const localeName = (code) => (code === 'nl' ? 'Nederlands' : 'English');
    return `
      <div class="settings-group-title">${esc(t('settings.booking.types'))}</div>
      <div class="card">
        ${types.map((type, i) => row({ title: esc(type.title), desc: esc(typeDesc(type)), action: `booking-type:${i}` })).join('')}
        ${types.length < MAX_TYPES ? row({ title: esc(t('settings.booking.addType')), action: 'booking-type:new', cls: 'indent', lead: `<span class="add-ic">${icons.plus}</span>` }) : ''}
      </div>
      <div class="settings-group-title">${esc(t('settings.booking.availability'))}</div>
      <div class="card">
        ${row({ title: esc(t('settings.booking.hours')), value: esc(hoursValue()), action: 'booking-hours' })}
        ${row({ title: esc(t('settings.booking.checked')), value: esc(t('settings.booking.checkedCount', { count: counted })), action: 'calset-open' })}
        ${row({ title: esc(t('settings.booking.destination')), desc: esc(destinationNote(dest)), value: esc(destName), action: 'booking-destination' })}
        ${row({ title: esc(t('settings.booking.dailyCap')), desc: esc(t('settings.booking.dailyCapHelp')), value: esc(String(s.dailyCap || 20)), action: 'booking-cap' })}
      </div>
      <div class="settings-group-title">${esc(t('settings.booking.page'))}</div>
      <div class="card">
        ${row({ title: esc(t('settings.booking.name')), value: esc(s.name || ''), action: 'booking-name' })}
        ${row({ title: esc(t('settings.booking.intro')), value: esc(s.intro || t('common.values.none')), action: 'booking-intro' })}
        ${row({ title: esc(t('settings.booking.contact')), desc: esc(t('settings.booking.contactHelp')), value: esc(s.contact || t('common.values.none')), action: 'booking-contact' })}
        ${row({ title: esc(t('settings.booking.zone')), value: esc(String(s.timeZone || '').replace(/_/g, ' ')), action: 'booking-zone' })}
        ${row({ title: esc(t('settings.booking.pageLanguage')), desc: esc(t('settings.booking.pageLanguageHelp')), value: esc(localeName(s.locale)), action: 'booking-locale' })}
        ${row({ title: esc(t('settings.booking.handle')), value: esc(s.handle || state.handle || ''), action: 'booking-handle' })}
        ${footerRow((app && app.settings) || {})}
      </div>`;
  }

  // ---------- meeting type ----------

  // A section of the meeting type editor under a heading that folds it. Each keeps the state the user left it in
  // while the editor is open: When guests can book starts open, More options folded.
  const section = (name, title, body) =>
    `<details class="booking-section booking-${name}" data-section="${name}" ${edit.open[name] ? 'open' : ''}><summary>${icons.chevronRight}<span>${esc(title)}</span></summary>
        ${body}
      </details>`;

  function typeHtml() {
    const d = edit.draft;
    const meet = meetAllowed();
    const loc = d.location;
    const showValue = loc.kind !== 'none' && loc.kind !== 'googleMeet' && loc.kind !== 'video';
    // Connected video services come right after Google Meet.
    const option = (value, label, extra = '') => `<option value="${esc(value)}"${value === kindValue(loc) ? ' selected' : ''}${extra}>${esc(label)}</option>`;
    const kinds = KINDS.map(
      (k) =>
        option(k, KIND_LABEL[k](), k === 'googleMeet' && !meet && loc.kind !== 'googleMeet' ? ' disabled' : '') +
        (k === 'googleMeet' ? videoOptions(loc.kind === 'video' ? loc.service : null).map((s) => option(`video:${s.id}`, videoLabel(s))).join('') : '')
    ).join('');
    // Every length the editor offers, with any the type has that isn't among them.
    const picked = lengthsOf(d);
    const lengths = [...new Set([...LENGTHS, ...picked])].sort((a, b) => a - b);
    const base = (state.url || '').replace(/\/$/, '');
    return `<div class="card"><form class="form booking-type" data-form="booking-type" novalidate>
      <label>${esc(t('settings.booking.typeTitle'))}<input type="text" name="title" value="${esc(d.title)}" maxlength="100" autocomplete="off"/></label>
      <label>${esc(t('settings.booking.typeSlug'))}<input type="text" name="slug" value="${esc(d.slug)}" maxlength="40" spellcheck="false" autocomplete="off" aria-describedby="booking-slug-url"/>
        <span class="field-help" id="booking-slug-url" data-slug-url>${esc(`${base}/${d.slug}`)}</span></label>
      <fieldset class="booking-lengths" aria-describedby="booking-lengths-help">
        <legend>${esc(t('settings.booking.typeLengths'))}</legend>
        <div class="booking-length-options">${lengths
          .map((m) => `<label class="booking-length"><input type="checkbox" name="durations" value="${m}"${picked.includes(m) ? ' checked' : ''}/><span>${esc(duration(m * 60000))}</span></label>`)
          .join('')}</div>
        <span class="field-help" id="booking-lengths-help">${esc(t('settings.booking.typeLengthsHelp'))}</span>
      </fieldset>
      <label data-default-length ${picked.length > 1 ? '' : 'hidden'}>${esc(t('settings.booking.typeDefaultLength'))}<select name="duration" aria-describedby="booking-default-help">${defaultOptions(picked, d.duration)}</select>
        <span class="field-help" id="booking-default-help">${esc(t('settings.booking.typeDefaultLengthHelp'))}</span></label>
      <label>${esc(t('settings.booking.typeDescription'))}<textarea name="description" maxlength="500" rows="3">${esc(d.description)}</textarea></label>
      <label>${esc(t('settings.booking.location'))}<select name="locationKind">${kinds}</select>
        ${meet ? '' : `<span class="field-help">${esc(t('settings.booking.meetUnavailable'))}</span>`}
        <span class="field-help" data-video-help>${esc(videoHelp(kindValue(loc)))}</span></label>
      <div class="booking-location" data-location ${showValue ? '' : 'hidden'}>
        <label>${esc(t('settings.booking.locationValue'))}<input type="text" name="locationValue" value="${esc(loc.value)}" maxlength="500" autocomplete="off" placeholder="${esc(showValue ? KIND_VALUE[loc.kind]() : '')}"/></label>
        <label class="inline"><input type="checkbox" name="locationPublic" ${loc.public ? 'checked' : ''}/> ${esc(t('settings.booking.locationPublic'))}</label>
        <span class="field-help">${esc(t('settings.booking.locationPrivateHelp'))}</span>
      </div>
      <label data-location-label ${loc.kind === 'googleMeet' ? 'hidden' : ''}>${esc(t('settings.booking.locationLabel'))}<input type="text" name="locationLabel" value="${esc(loc.label)}" maxlength="100" autocomplete="off" placeholder="${esc(labelPlaceholder(kindValue(loc)))}"/></label>
      ${section('when', t('settings.booking.when'), `<div class="booking-grid">
          <label>${esc(t('settings.booking.bufferBefore'))}<select name="bufferBefore" aria-describedby="booking-buffer-help">${options(BUFFERS.includes(d.bufferBefore) ? BUFFERS : [...BUFFERS, d.bufferBefore], d.bufferBefore, (m) => (m ? duration(m * 60000) : t('common.values.none')))}</select></label>
          <label>${esc(t('settings.booking.bufferAfter'))}<select name="bufferAfter" aria-describedby="booking-buffer-help">${options(BUFFERS.includes(d.bufferAfter) ? BUFFERS : [...BUFFERS, d.bufferAfter], d.bufferAfter, (m) => (m ? duration(m * 60000) : t('common.values.none')))}</select></label>
          <span class="field-help booking-grid-help" id="booking-buffer-help">${esc(t('settings.booking.bufferHelp'))}</span>
          <label>${esc(t('settings.booking.minNotice'))}<select name="minNotice">${options(NOTICES.includes(d.minNotice) ? NOTICES : [...NOTICES, d.minNotice].sort((a, b) => a - b), d.minNotice, noticeLength)}</select></label>
          <label>${esc(t('settings.booking.horizon'))}<select name="horizon">${options(HORIZONS.includes(d.horizon) ? HORIZONS : [...HORIZONS, d.horizon].sort((a, b) => a - b), d.horizon, (n) => t('settings.booking.days', { count: n }))}</select></label>
          <label>${esc(t('settings.booking.slotInterval'))}<select name="slotInterval"><option value=""${d.slotInterval === null ? ' selected' : ''}>${esc(t('settings.booking.slotSame'))}</option>${options(
            INTERVALS.includes(d.slotInterval) || d.slotInterval === null ? INTERVALS : [...INTERVALS, d.slotInterval],
            d.slotInterval ?? '',
            (m) => duration(m * 60000)
          )}</select></label>
          <label>${esc(t('settings.booking.dailyLimit'))}<select name="dailyLimit"><option value=""${d.dailyLimit === null ? ' selected' : ''}>${esc(t('settings.booking.noLimit'))}</option>${options(
            LIMITS.includes(d.dailyLimit) || d.dailyLimit === null ? LIMITS : [...LIMITS, d.dailyLimit],
            d.dailyLimit ?? '',
            (n) => t('settings.booking.perDay', { count: n })
          )}</select></label>
        </div>`)}
      ${section('more', t('settings.booking.more'), `<label class="inline"><input type="checkbox" name="confirm" ${d.confirm ? 'checked' : ''}/> ${esc(t('settings.booking.typeConfirm'))}</label>
        <span class="field-help">${esc(t('settings.booking.confirmHelp'))}</span>
        <label class="inline"><input type="checkbox" name="hidden" ${d.hidden ? 'checked' : ''}/> ${esc(t('settings.booking.hide'))}</label>
        <span class="field-help">${esc(t('settings.booking.hideHelp'))}</span>`)}
      <div class="error" data-error role="alert"></div>
      <div class="booking-form-actions">
        <button class="btn primary" type="submit">${esc(t('common.actions.save'))}</button>
        <button class="btn" type="button" data-a="back">${esc(t('common.actions.cancel'))}</button>
        <span class="spacer"></span>
        ${edit.index !== null && (state.settings.types || []).length > 1 ? `<button class="btn" type="button" data-a="booking-type-delete"><span class="danger-text">${esc(t('settings.booking.deleteType'))}</span></button>` : ''}
      </div>
    </form></div>`;
  }

  // Under Location: what a video service does, or where to connect one while none is.
  function videoHelp(value) {
    if (value.startsWith('video:')) return t('settings.booking.videoHelp', { service: videoName(value.slice(6)) });
    if (!state.thisDevice) return t('settings.booking.videoOtherDevice');
    return videoOptions(null).length ? '' : t('settings.booking.videoNone');
  }
  const labelPlaceholder = (value) => (value.startsWith('video:') ? videoName(value.slice(6)) : t('settings.booking.locationLabelPlaceholder'));

  // What the type form holds now, as a meeting type.
  function readType(form) {
    const f = new FormData(form);
    const num = (name, fallback) => (f.get(name) === '' || f.get(name) === null ? fallback : Number(f.get(name)));
    const choice = String(f.get('locationKind') || 'none');
    const service = choice.startsWith('video:') ? choice.slice(6) : null;
    const kind = service ? 'video' : choice;
    const label = String(f.get('locationLabel') || '').trim();
    const durations = f
      .getAll('durations')
      .map(Number)
      .sort((a, b) => a - b);
    const chosen = num('duration', durations[0]);
    return {
      ...edit.draft,
      title: String(f.get('title') || '').trim(),
      slug: String(f.get('slug') || '').trim(),
      durations,
      // The default is one of the lengths picked; the field follows them as they change (fill()).
      duration: durations.includes(chosen) ? chosen : durations[0],
      description: String(f.get('description') || '').trim(),
      // A video service's link is made per booking, so there are no details to keep or show.
      location: service
        ? { kind, label, value: '', public: false, service }
        : {
            kind,
            label,
            value: kind === 'none' || kind === 'googleMeet' ? '' : String(f.get('locationValue') || '').trim(),
            public: kind !== 'none' && kind !== 'googleMeet' && f.get('locationPublic') === 'on'
          },
      bufferBefore: num('bufferBefore', 0),
      bufferAfter: num('bufferAfter', 0),
      minNotice: num('minNotice', 240),
      horizon: num('horizon', 30),
      slotInterval: num('slotInterval', null),
      dailyLimit: num('dailyLimit', null),
      confirm: f.get('confirm') === 'on',
      hidden: f.get('hidden') === 'on'
    };
  }

  // ---------- hours ----------

  function hoursDraft() {
    if (hours) return hours;
    const s = state.settings || {};
    const c = (app && app.settings && app.settings.calendar) || { workDays: [1, 2, 3, 4, 5], workStart: '09:00', workEnd: '17:00' };
    const working = Array.from({ length: 7 }, (_, d) => (c.workDays.includes(d) ? [[c.workStart, c.workEnd]] : []));
    hours = {
      custom: Boolean(s.hours && s.hours.custom),
      weekly: s.hours && Array.isArray(s.hours.weekly) && s.hours.weekly.length === 7 ? clone(s.hours.weekly) : working,
      overrides: clone(s.overrides || {}),
      removed: []
    };
    return hours;
  }

  const rangeHtml = (prefix, ranges, label) =>
    ranges
      .map(
        ([from, to], i) => `<span class="booking-range">
          <input type="time" step="300" data-range="${prefix}:${i}:0" value="${esc(from)}" aria-label="${esc(t('settings.booking.rangeFrom', { day: label }))}"/>
          <span aria-hidden="true">–</span>
          <input type="time" step="300" data-range="${prefix}:${i}:1" value="${esc(to === '24:00' ? '00:00' : to)}" aria-label="${esc(t('settings.booking.rangeTo', { day: label }))}"/>
          <button type="button" class="icon-btn xs" data-a="booking-range-del:${prefix}:${i}" title="${esc(t('settings.booking.removeRange'))}" aria-label="${esc(t('settings.booking.removeRange'))}">${icons.close}</button></span>`
      )
      .join('');

  function hoursHtml() {
    const h = hoursDraft();
    const calendar = app && app.settings && app.settings.calendar;
    const dates = Object.keys(h.overrides).sort();
    const week = weekOrder(calendar)
      .map((d) => {
        const ranges = h.weekly[d] || [];
        return `<div class="row static booking-day">
          <div class="booking-day-name">${esc(weekdayName(d))}</div>
          <div class="booking-ranges">${ranges.length ? rangeHtml(`w${d}`, ranges, weekdayName(d)) : `<span class="booking-unavailable">${esc(t('settings.booking.unavailableDay'))}</span>`}</div>
          <button type="button" class="icon-btn sm" data-a="booking-range-add:w${d}" title="${esc(t('settings.booking.addRange', { day: weekdayName(d, 'long', true) }))}" aria-label="${esc(t('settings.booking.addRange', { day: weekdayName(d, 'long', true) }))}" ${ranges.length >= MAX_RANGES || !roomFor(ranges) ? 'disabled' : ''}>${icons.plus}</button>
        </div>`;
      })
      .join('');
    const overrides = dates
      .map((date) => {
        const ranges = h.overrides[date];
        const label = longDay(new Date(`${date}T12:00:00`).getTime());
        return `<div class="row static booking-day">
          <div class="booking-day-name">${esc(label)}</div>
          <div class="booking-ranges">${ranges.length ? rangeHtml(`o${date}`, ranges, label) : `<span class="booking-blocked">${esc(t('settings.booking.blockedDay'))}</span>`}</div>
          <button type="button" class="icon-btn sm" data-a="booking-override-del:${esc(date)}" title="${esc(t('settings.booking.removeOverride', { date: label }))}" aria-label="${esc(t('settings.booking.removeOverride', { date: label }))}">${icons.trash}</button>
        </div>`;
      })
      .join('');
    return `
      <div class="card">${row({ title: esc(t('settings.booking.hoursCustomToggle')), desc: esc(h.custom ? t('settings.booking.hoursCustomHelp') : t('settings.booking.hoursWorkingHelp', { hours: workHoursText(calendar) })), action: 'booking-hours-custom', toggle: h.custom })}</div>
      <form class="booking-hours-form" data-form="booking-hours" novalidate>
        ${h.custom ? `<div class="settings-group-title">${esc(t('settings.booking.weekly'))}</div><div class="card booking-week">${week}</div>` : ''}
        <div class="settings-group-title">${esc(t('settings.booking.overrides'))}</div>
        <p class="settings-text">${esc(t('settings.booking.overridesHelp'))}</p>
        <div class="card">${overrides}
          <button type="button" class="row indent" data-a="booking-override-add"><span class="add-ic">${icons.plus}</span><div class="text"><div class="title">${esc(t('settings.booking.addOverride'))}</div></div></button>
        </div>
        <div class="error booking-form-error" data-error role="alert"></div>
        <div class="booking-form-actions">
          <button class="btn primary" type="submit">${esc(t('common.actions.save'))}</button>
          <button class="btn" type="button" data-a="back">${esc(t('common.actions.cancel'))}</button>
        </div>
      </form>`;
  }

  // Ranges sorted, each ending after it starts, none overlapping, at most six; '00:00' as an end is midnight.
  function cleanRanges(ranges) {
    const out = ranges.map(([a, b]) => [a, b === '00:00' ? '24:00' : b]).sort((x, y) => (x[0] < y[0] ? -1 : 1));
    if (out.length > MAX_RANGES) return null;
    for (let i = 0; i < out.length; i++) {
      const [a, b] = out[i];
      if (!/^\d\d:\d\d$/.test(a) || !/^\d\d:\d\d$/.test(b) || b <= a) return null;
      if (i && out[i - 1][1] > a) return null;
    }
    return out;
  }

  // Where another range fits in a day: an hour, or what is left of one, after the last range, else before the first,
  // else in a gap between two; null when the day is full. Nine to ten on a day without hours.
  function roomFor(ranges) {
    const minutes = (v) => (/^\d\d:\d\d$/.test(v) ? Number(v.slice(0, 2)) * 60 + Number(v.slice(3)) : NaN);
    const text = (m) => (m >= 1440 ? '24:00' : `${String(Math.floor(m / 60)).padStart(2, '0')}:${String(m % 60).padStart(2, '0')}`);
    const list = ranges
      .map(([a, b]) => [minutes(a), b === '00:00' || b === '24:00' ? 1440 : minutes(b)])
      .filter(([a, b]) => Number.isFinite(a) && Number.isFinite(b))
      .sort((x, y) => x[0] - y[0]);
    if (!list.length) return ['09:00', '10:00'];
    const end = Math.max(...list.map((r) => r[1]));
    if (end < 1440) return [text(end), text(Math.min(end + 60, 1440))];
    if (list[0][0] > 0) return [text(Math.max(list[0][0] - 60, 0)), text(list[0][0])];
    for (let i = 1; i < list.length; i++) if (list[i - 1][1] < list[i][0]) return [text(list[i - 1][1]), text(Math.min(list[i - 1][1] + 60, list[i][0]))];
    return null;
  }

  function addOverrideDialog() {
    const today = new Date();
    const min = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, '0')}-${String(today.getDate()).padStart(2, '0')}`;
    return dialog({
      title: t('settings.booking.addOverride'),
      buttons: [
        { get label() { return t('common.actions.cancel'); }, value: null },
        {
          get label() { return t('common.actions.add'); },
          primary: true,
          value: (scrim) => {
            const date = scrim.querySelector('[name="date"]').value;
            const blocked = scrim.querySelector('[name="kind"]:checked').value === 'block';
            const from = scrim.querySelector('[name="from"]').value;
            const to = scrim.querySelector('[name="to"]').value;
            const error = scrim.querySelector('[data-error]');
            if (!/^\d{4}-\d\d-\d\d$/.test(date) || date < min) return (error.textContent = t('settings.booking.overrideDateNeeded')), KEEP_OPEN;
            if (!blocked && !cleanRanges([[from, to]])) return (error.textContent = t('settings.booking.rangeInvalid')), KEEP_OPEN;
            return { date, ranges: blocked ? [] : cleanRanges([[from, to]]) };
          }
        }
      ],
      render(body) {
        body.innerHTML = `<div class="form" style="padding:0">
          <label>${esc(t('settings.booking.overrideDate'))}<input type="date" name="date" min="${min}" value="${min}"/></label>
          <label class="inline"><input type="radio" name="kind" value="block" checked/> ${esc(t('settings.booking.overrideBlock'))}</label>
          <label class="inline"><input type="radio" name="kind" value="hours"/> ${esc(t('settings.booking.overrideHours'))}</label>
          <div class="booking-range"><input type="time" step="300" name="from" value="09:00" aria-label="${esc(t('settings.calendar.workFrom'))}"/><span aria-hidden="true">–</span><input type="time" step="300" name="to" value="12:00" aria-label="${esc(t('settings.calendar.workTo'))}"/></div>
          <div class="error" data-error></div></div>`;
      }
    });
  }

  // ---------- views ----------

  // { title, body } of a booking page; the title goes into the page bar as markup, so it is escaped here.
  function view(name) {
    const title = esc(t('settings.booking.title'));
    // Which screen this is, for screen(), and what the page shows, for onState().
    const drawn = (kind, out) => {
      screen = `${name}:${kind}`;
      drawnPage = kind === 'page' || kind === 'done' ? pageShown(state) : null;
      return out;
    };
    if (!state) return drawn('none', { title, body: failed ? `<p class="settings-text error">${esc(t('settings.booking.loadFailed', { message: failed }))}</p>` : '' });
    if (state.status === 'signed_out') return drawn('signed-out', { title, body: signedOutHtml() });
    if (state.status === 'unsupported') return drawn('unsupported', { title, body: unsupportedHtml() });
    if (name === 'booking-type' && edit && ready()) return drawn(`type-${edit.index}`, { title: esc(edit.index === null ? t('settings.booking.typeNew') : edit.draft.title || t('settings.booking.title')), body: typeHtml() });
    if (name === 'booking-hours' && ready()) return drawn('hours', { title: esc(t('settings.booking.hours')), body: hoursHtml() });
    if (name === 'booking-setup' || !isOn()) return drawn('setup', { title: name === 'booking-setup' ? esc(t('settings.booking.setupTitle')) : title, body: setupHtml() });
    return drawn(justOn ? 'done' : 'page', { title, body: pageHtml() });
  }

  // After a page is drawn: what its fields do while the user types.
  function fill(name, page) {
    host = page;
    // "Last updated 3 minutes ago" goes on counting: main sends the state only when it changes.
    clearInterval(clock);
    clock = null;
    if (page.querySelector('[data-booking-publish]')) {
      clock = setInterval(() => {
        if (current() === 'booking' && host.isConnected && host.querySelector('[data-booking-publish]')) showPublish();
        else clearInterval(clock);
      }, 30000);
    }
    const setupForm = page.querySelector('form[data-form="booking-setup"]');
    if (setupForm) {
      const v = setupValues();
      setupForm.addEventListener('input', (e) => {
        const el = e.target;
        if (el.name === 'handle') {
          v.handle = el.value.trim();
          el.removeAttribute('aria-invalid');
          checkHandle(v.handle, page);
        }
        if (el.name === 'name') v.name = el.value;
      });
      setupForm.addEventListener('change', (e) => {
        const el = e.target;
        if (el.name === 'timeZone') v.timeZone = el.value;
        if (el.name === 'cal') v.calendars[el.value] = el.checked;
        if (el.name === 'destination') {
          v.destination = el.value;
          const note = page.querySelector('[data-destination-note]');
          if (note) note.textContent = destinationNote(destinations().find((c) => c.key === el.value));
        }
      });
      checkHandle(v.handle, page);
    }
    const typeForm = page.querySelector('form[data-form="booking-type"]');
    if (typeForm && edit) {
      const slugUrl = page.querySelector('[data-slug-url]');
      const base = (state.url || '').replace(/\/$/, '');
      typeForm.addEventListener('input', (e) => {
        const el = e.target;
        if (el.name === 'title' && edit.index === null && !edit.slugTouched) typeForm.querySelector('[name="slug"]').value = slugify(el.value, 40);
        if (el.name === 'slug') edit.slugTouched = true;
        el.removeAttribute('aria-invalid');
        if (slugUrl) slugUrl.textContent = `${base}/${typeForm.querySelector('[name="slug"]').value.trim()}`;
      });
      typeForm.addEventListener('change', (e) => {
        if (e.target.name !== 'locationKind') return;
        const kind = e.target.value;
        const showValue = kind !== 'none' && kind !== 'googleMeet' && !kind.startsWith('video:');
        typeForm.querySelector('[data-location]').hidden = !showValue;
        typeForm.querySelector('[data-location-label]').hidden = kind === 'googleMeet';
        if (showValue) typeForm.querySelector('[name="locationValue"]').placeholder = KIND_VALUE[kind]();
        typeForm.querySelector('[name="locationLabel"]').placeholder = labelPlaceholder(kind);
        typeForm.querySelector('[data-video-help]').textContent = videoHelp(kind);
      });
      for (const el of typeForm.querySelectorAll('details[data-section]')) el.addEventListener('toggle', () => edit && (edit.open[el.dataset.section] = el.open));
      // Available durations: at least one stays picked, Default duration lists the ones picked and shows only with
      // several, and a title Rukoo gave follows them ("30 minute meeting", "Meeting").
      let shown = lengthsOf(edit.draft);
      typeForm.addEventListener('change', (e) => {
        if (e.target.name !== 'durations') return;
        const boxes = [...typeForm.querySelectorAll('[name="durations"]')];
        const error = typeForm.querySelector('[data-error]');
        if (!boxes.some((b) => b.checked)) {
          e.target.checked = true;
          if (error) error.textContent = t('settings.booking.lengthsNeeded');
          return;
        }
        if (error && error.textContent === t('settings.booking.lengthsNeeded')) error.textContent = '';
        const next = boxes.filter((b) => b.checked).map((b) => Number(b.value));
        const select = typeForm.querySelector('[name="duration"]');
        const old = Number(select.value);
        select.innerHTML = defaultOptions(next, next.includes(old) ? old : nearestLength(next, old));
        typeForm.querySelector('[data-default-length]').hidden = next.length < 2;
        const locale = (state.settings && state.settings.locale) || getLanguage();
        const title = typeForm.querySelector('[name="title"]');
        if (isMadeTitle(locale, title.value.trim(), shown)) title.value = madeTitle(locale, next);
        shown = next;
      });
    }
    const hoursForm = page.querySelector('form[data-form="booking-hours"]');
    if (hoursForm && hours) {
      hoursForm.addEventListener('input', (e) => {
        const at = e.target.dataset.range;
        if (!at) return;
        const [where, i, end] = at.split(':');
        const list = where[0] === 'w' ? hours.weekly[Number(where.slice(1))] : hours.overrides[where.slice(1)];
        if (list && list[Number(i)]) list[Number(i)][Number(end)] = e.target.value;
      });
    }
  }

  // ---------- actions ----------

  // A field main refused (contracts f: booking errors carry code and field) is marked and gets the focus.
  function fail(form, err) {
    const error = form.querySelector('[data-error]');
    if (error) error.textContent = err.message;
    const field = String(err.field || '').split('.').pop();
    const names = { location: 'locationValue', value: 'locationValue', kind: 'locationKind', service: 'locationKind', label: 'locationLabel', public: 'locationPublic' };
    const input = field && form.querySelector(`[name="${CSS.escape(names[field] || field)}"]`);
    if (input) {
      input.setAttribute('aria-invalid', 'true');
      if (input.closest('details')) input.closest('details').open = true;
      input.focus();
    }
  }

  async function busy(form, label, work) {
    const btn = form.querySelector('button[type=submit]');
    const text = btn.textContent;
    btn.disabled = true;
    btn.innerHTML = `<span class="spinner"></span>${esc(label)}`;
    try {
      return await work();
    } finally {
      if (btn.isConnected) {
        btn.disabled = false;
        btn.textContent = text;
      }
    }
  }

  async function submit(form) {
    const kind = form.dataset.form;
    form.querySelector('[data-error]').textContent = '';
    form.querySelectorAll('[aria-invalid]').forEach((el) => el.removeAttribute('aria-invalid'));
    if (kind === 'booking-setup') return submitSetup(form);
    if (kind === 'booking-type') return submitType(form);
    if (kind === 'booking-hours') return submitHours(form);
  }

  async function submitSetup(form) {
    const v = setupValues();
    const f = new FormData(form);
    v.handle = String(f.get('handle') || '').trim();
    v.name = String(f.get('name') || '').trim();
    v.timeZone = String(f.get('timeZone') || pcZone());
    v.destination = String(f.get('destination') || '');
    const stop = (field, message) => fail(form, { field, message });
    if (!HANDLE.test(v.handle)) return stop('handle', t('settings.booking.handleHelp'));
    if (RESERVED_HANDLES.has(v.handle)) return stop('handle', t('settings.booking.handleReserved'));
    if (!v.name) return stop('name', t('settings.booking.nameNeeded'));
    if (!v.destination) return stop('destination', t('settings.booking.noDestination'));
    try {
      const next = await busy(form, t('settings.booking.turningOn'), async () => {
        // The calendars go first, so the first publish already leaves out their busy time (contracts g).
        const availability = {};
        for (const box of form.querySelectorAll('[name="cal"]')) availability[box.value] = box.checked;
        if (Object.keys(availability).length) await api('calendarSettings', { availability });
        const s = state.settings || {};
        return api('bookingEnable', {
          handle: v.handle,
          name: v.name,
          timeZone: v.timeZone,
          hours: s.hours && typeof s.hours === 'object' ? s.hours : { custom: false, weekly: null },
          destination: v.destination
        });
      });
      state = next;
      justOn = true;
      setup = null;
      if (current() === 'booking-setup') back();
      else rerender();
    } catch (err) {
      fail(form, err);
    }
  }

  async function save(patch, done) {
    try {
      state = await api('bookingSave', patch);
      if (done) toast(done);
      return true;
    } catch (err) {
      toast(err.message, 6000);
      return false;
    }
  }

  async function submitType(form) {
    const type = readType(form);
    const stop = (field, message) => fail(form, { field, message });
    if (!type.title) return stop('title', t('settings.booking.titleNeeded'));
    if (!type.durations.length) return stop('durations', t('settings.booking.lengthsNeeded'));
    if (!SLUG.test(type.slug) || RESERVED_SLUGS.has(type.slug)) return stop('slug', t('settings.booking.slugHelp'));
    const types = clone(state.settings.types || []);
    if (types.some((x, i) => x.slug === type.slug && i !== edit.index)) return stop('slug', t('settings.booking.slugTaken'));
    if (type.location.kind === 'link' && type.location.value && !/^https:\/\//i.test(type.location.value)) return stop('locationValue', t('settings.booking.linkHttps'));
    if (edit.index === null) types.push(type);
    else types[edit.index] = type;
    try {
      state = await busy(form, t('common.status.saving'), () => api('bookingSave', { types }));
      edit = null;
      toast(t('settings.booking.saved'));
      back();
    } catch (err) {
      fail(form, err);
    }
  }

  async function submitHours(form) {
    const h = hours;
    const weekly = [];
    for (let d = 0; d < 7; d++) {
      const ranges = cleanRanges(h.weekly[d] || []);
      if (!ranges) return fail(form, { message: t('settings.booking.rangeInvalid', {}), field: '' });
      weekly.push(ranges);
    }
    const overrides = {};
    for (const [date, list] of Object.entries(h.overrides)) {
      const ranges = cleanRanges(list);
      if (!ranges) return fail(form, { message: t('settings.booking.rangeInvalid'), field: '' });
      overrides[date] = ranges;
    }
    for (const date of h.removed) if (!(date in overrides)) overrides[date] = null;
    try {
      state = await busy(form, t('common.status.saving'), () => api('bookingSave', { hours: { custom: h.custom, weekly: h.custom ? weekly : null }, overrides }));
      hours = null;
      toast(t('settings.booking.saved'));
      back();
    } catch (err) {
      fail(form, err);
    }
  }

  async function copyLink() {
    if (!state || !state.url) return;
    try {
      await navigator.clipboard.writeText(state.url);
      toast(t('settings.booking.copied'));
    } catch (err) {
      toast(err.message, 5000);
    }
  }

  async function action(work, done) {
    try {
      const next = await work();
      if (next && typeof next === 'object' && next.status) state = next;
      if (done) toast(done);
    } catch (err) {
      toast(err.message, 6000);
    }
    rerender();
  }

  // A message to the guest for a decline or a cancellation, with the choice that goes with it.
  function messageDialog({ title, help, okLabel, check: box }) {
    return dialog({
      title,
      buttons: [
        { get label() { return t('settings.booking.keep'); }, value: null },
        {
          label: okLabel,
          danger: true,
          value: (scrim) => ({ message: scrim.querySelector('textarea').value.trim().slice(0, 500), checked: box ? scrim.querySelector('[name="check"]').checked : false })
        }
      ],
      render(body) {
        body.innerHTML = `<div class="form" style="padding:0">${help ? `<p>${esc(help)}</p>` : ''}
          <label>${esc(t('settings.booking.message'))}<textarea maxlength="500" rows="3"></textarea></label>
          ${box ? `<label class="inline"><input type="checkbox" name="check" ${box.on ? 'checked' : ''}/> ${esc(box.label)}</label>` : ''}</div>`;
      }
    });
  }

  // Cancelling a booking, from the list or a notice: the host confirms, may write the guest a message and chooses
  // whether the calendar tells the guest.
  async function cancelAsk(id) {
    const v = await messageDialog({
      title: t('settings.booking.cancelTitle'),
      help: t('settings.booking.cancelHelp'),
      okLabel: t('settings.booking.cancel'),
      check: { label: t('settings.booking.cancelNotify'), on: true }
    });
    if (v) await action(() => api('bookingAction', id, 'cancel', { message: v.message || undefined, notify: v.checked }), t('settings.booking.cancelled'));
  }

  async function noticeAction(id, name) {
    const n = (state.notices || []).find((x) => x.id === id);
    const a = n && (n.actions || []).find((x) => x.action === name);
    if (!a) return;
    switch (a.action) {
      case 'takeover':
        return action(() => api('bookingTakeover'), t('settings.booking.takenOver'));
      case 'video':
        return go('video');
      case 'cancel':
        return n.bookingId ? cancelAsk(n.bookingId) : null;
      case 'keep':
      case 'dismiss':
        return action(() => api('bookingAction', n.id, a.action));
      case 'reconnect': {
        const acc = ((app && app.accounts) || []).find((x) => x.id === n.accountId);
        if (!acc) return;
        if (acc.provider !== 'google') return go('server', { id: acc.id });
        toast(t('setup.google.waitToast'), 60000);
        return action(() => api('googleReauth', acc.id), t('setup.google.signedIn'));
      }
    }
  }

  async function zoneDialog(value) {
    return dialog({
      title: t('settings.booking.zone'),
      buttons: [
        { get label() { return t('common.actions.cancel'); }, value: null },
        { get label() { return t('common.actions.save'); }, primary: true, value: (scrim) => scrim.querySelector('select').value }
      ],
      render(body) {
        body.innerHTML = `<div class="form" style="padding:0"><select aria-label="${esc(t('settings.booking.zone'))}">${zoneOptions(value)}</select></div>`;
        setTimeout(() => body.querySelector('select').focus(), 0);
      }
    });
  }

  // Handles a data-a action of these pages; false when it isn't one.
  async function click(a) {
    if (a === 'booking-open') return go('booking'), true;
    if (a.startsWith('booking-account:')) return go(a.slice(16)), true;
    // These change the settings record. Without it on this computer the page doesn't offer them (pageHtml); a click
    // already on its way when the page turned off or the record went is answered here.
    if (/^booking-(type|cap|name|intro|contact|zone|locale|handle|hours|destination|override|range)/.test(a) && !ready()) {
      if (isOn()) toast(t('settings.booking.settingsMissing'), 6000);
      return true;
    }
    if (a.startsWith('booking-acc-footer:')) {
      const id = decodeURIComponent(a.slice(19));
      const acc = ((app && app.accounts) || []).find((x) => x.id === id);
      if (!acc) return true;
      const own = typeof acc.bookingFooter === 'boolean';
      const current = !own ? 'general' : acc.bookingFooter ? (acc.bookingFooterText ? 'text' : 'on') : 'off';
      const v = await choiceDialog(
        t('settings.footer.title'),
        [
          { value: 'general', label: t('settings.footer.choiceGeneral') },
          { value: 'on', label: t('settings.footer.choiceOn') },
          { value: 'off', label: t('settings.footer.choiceOff') },
          { value: 'text', label: t('settings.footer.choiceText') }
        ],
        current
      );
      if (!v) return true;
      let patch = v === 'general' ? { bookingFooter: null } : { bookingFooter: v !== 'off', bookingFooterText: null };
      if (v === 'text') {
        const text = await footerTextDialog(acc.bookingFooterText || (app.settings && app.settings.bookingFooterText));
        if (!text) return true;
        patch = { bookingFooter: true, bookingFooterText: text === 'default' ? null : text };
      }
      try {
        await api('updateAccount', id, patch);
      } catch (err) {
        toast(err.message, 5000);
      }
      await rerender();
      return true;
    }
    if (a.startsWith('booking-type:')) {
      const which = a.slice(13);
      const types = state.settings.types || [];
      const index = which === 'new' ? null : Number(which);
      if (index !== null && !types[index]) return true;
      edit = { index, draft: index === null ? defaultType(meetAllowed()) : clone(types[index]), slugTouched: index !== null, open: { when: true, more: false } };
      go('booking-type');
      return true;
    }
    if (a.startsWith('booking-accept:')) {
      await action(() => api('bookingAction', a.slice(15), 'accept'), t('settings.booking.accepted'));
      return true;
    }
    if (a.startsWith('booking-decline:')) {
      const id = a.slice(16);
      const dest = currentDestination();
      const v = await messageDialog({
        title: t('settings.booking.declineTitle'),
        okLabel: t('settings.booking.decline'),
        check: dest && dest.account.kind !== 'demo' ? { label: t('settings.booking.declineEmail', { address: dest.account.email }), on: false } : null
      });
      if (v) await action(() => api('bookingAction', id, 'decline', { message: v.message || undefined, email: v.checked }), t('settings.booking.declined'));
      return true;
    }
    if (a.startsWith('booking-cancel:')) {
      await cancelAsk(a.slice(15));
      return true;
    }
    if (a.startsWith('booking-notice:')) {
      const [, id, name] = a.split(':');
      await noticeAction(decodeURIComponent(id || ''), decodeURIComponent(name || ''));
      return true;
    }
    if (a.startsWith('booking-range-add:') || a.startsWith('booking-range-del:')) {
      if (!hours) return true;
      const [, where, i] = a.split(':');
      const list = where[0] === 'w' ? hours.weekly[Number(where.slice(1))] : hours.overrides[where.slice(1)];
      if (!list) return true;
      if (a.startsWith('booking-range-add:')) {
        // The times typed since the last redraw count: they are in the draft already.
        const room = roomFor(list);
        if (!room || list.length >= MAX_RANGES) return true;
        list.push(room);
        list.sort((x, y) => (x[0] < y[0] ? -1 : 1));
      } else list.splice(Number(i), 1);
      await rerender();
      return true;
    }
    if (a.startsWith('booking-override-del:')) {
      if (!hours) return true;
      const date = a.slice(21);
      delete hours.overrides[date];
      if (state.settings.overrides && date in state.settings.overrides) hours.removed.push(date);
      await rerender();
      return true;
    }
    switch (a) {
      case 'booking-copy':
        await copyLink();
        return true;
      case 'booking-preview':
        if (state && state.url) api('openExternal', state.url);
        return true;
      case 'booking-signature':
        try {
          await api('updateSettings', { bookingFooter: true });
          if (app && app.settings) app.settings.bookingFooter = true;
          toast(t('settings.booking.addedToSignature'));
        } catch (err) {
          toast(err.message, 5000);
        }
        await rerender();
        return true;
      case 'booking-footer': {
        const s = (app && app.settings) || {};
        try {
          await api('updateSettings', { bookingFooter: !s.bookingFooter });
        } catch (err) {
          toast(err.message, 5000);
        }
        await rerender();
        return true;
      }
      case 'booking-footer-text': {
        const s = (app && app.settings) || {};
        const v = await footerTextDialog(s.bookingFooterText);
        if (!v) return true;
        try {
          await api('updateSettings', { bookingFooterText: v === 'default' ? null : v });
        } catch (err) {
          toast(err.message, 5000);
        }
        await rerender();
        return true;
      }
      case 'booking-off': {
        const ok = await confirmDialog(t('settings.booking.offTitle'), t('settings.booking.offConfirm'), t('settings.booking.offAction'), true);
        if (ok) await action(() => api('bookingDisable'), t('settings.booking.turnedOff'));
        return true;
      }
      case 'booking-sync':
        await action(() => api('bookingSync'), t('settings.booking.synced'));
        return true;
      case 'booking-takeover':
        await action(() => api('bookingTakeover'), t('settings.booking.takenOver'));
        return true;
      case 'booking-hours':
        hours = null;
        hoursDraft();
        go('booking-hours');
        return true;
      case 'booking-hours-custom':
        if (hours) hours.custom = !hours.custom;
        await rerender();
        return true;
      case 'booking-override-add': {
        const v = await addOverrideDialog();
        if (v && hours) {
          hours.overrides[v.date] = v.ranges;
          hours.removed = hours.removed.filter((d) => d !== v.date);
          await rerender();
        }
        return true;
      }
      case 'booking-type-delete': {
        if (!edit || edit.index === null) return true;
        const type = state.settings.types[edit.index];
        const ok = await confirmDialog(t('settings.booking.deleteType'), t('settings.booking.deleteTypeConfirm', { title: type.title }), t('common.actions.delete'), true);
        if (!ok) return true;
        const types = clone(state.settings.types).filter((_, i) => i !== edit.index);
        if (await save({ types }, t('settings.booking.saved'))) {
          edit = null;
          back();
        }
        return true;
      }
      case 'booking-destination': {
        const dests = destinations();
        const now = currentDestination();
        const v = await choiceDialog(
          t('settings.booking.destination'),
          dests.map((c) => ({ value: c.portable, label: c.name, hint: c.account.email })),
          now ? now.portable : null
        );
        if (v && (await save({ destination: v }, t('settings.booking.saved')))) await rerender();
        return true;
      }
      case 'booking-cap': {
        const v = await promptDialog(t('settings.booking.dailyCap'), String(state.settings.dailyCap || 20));
        if (v === null) return true;
        const n = Number(String(v).trim());
        if (!Number.isInteger(n) || n < 1 || n > 50) {
          toast(t('settings.booking.dailyCapHelp'), 5000);
          return true;
        }
        if (await save({ dailyCap: n }, t('settings.booking.saved'))) await rerender();
        return true;
      }
      case 'booking-name':
      case 'booking-intro':
      case 'booking-contact': {
        const field = a.slice(8);
        const titles = { name: t('settings.booking.name'), intro: t('settings.booking.intro'), contact: t('settings.booking.contact') };
        const v = await promptDialog(titles[field], state.settings[field] || '', {
          multiline: field === 'intro',
          placeholder: field === 'contact' ? t('settings.booking.contactPlaceholder') : ''
        });
        if (v === null) return true;
        const text = v.trim();
        if (field === 'name' && !text) return toast(t('settings.booking.nameNeeded'), 5000), true;
        if (await save({ [field]: field === 'contact' ? text || null : text }, t('settings.booking.saved'))) await rerender();
        return true;
      }
      case 'booking-zone': {
        const v = await zoneDialog(state.settings.timeZone);
        if (v && v !== state.settings.timeZone && (await save({ timeZone: v }, t('settings.booking.saved')))) await rerender();
        return true;
      }
      case 'booking-locale': {
        const v = await choiceDialog(t('settings.booking.pageLanguage'), [{ value: 'en', label: 'English' }, { value: 'nl', label: 'Nederlands' }], state.settings.locale);
        if (v && v !== state.settings.locale && (await save({ locale: v }, t('settings.booking.saved')))) await rerender();
        return true;
      }
      case 'booking-handle': {
        const v = await promptDialog(t('settings.booking.handle'), state.settings.handle || '');
        if (v === null) return true;
        const handle = v.trim();
        if (handle === state.settings.handle) return true;
        if (!HANDLE.test(handle) || RESERVED_HANDLES.has(handle)) return toast(t('settings.booking.handleHelp'), 6000), true;
        if (await save({ handle }, t('settings.booking.saved'))) await rerender();
        return true;
      }
    }
    return false;
  }

  // Which booking screen was drawn last: settings.js keeps the scroll position while it stays the same.
  return { prepare, onState, rowHtml, footerRow, accountFooterRow, view, fill, submit, click, state: () => state, screen: () => screen };
}
