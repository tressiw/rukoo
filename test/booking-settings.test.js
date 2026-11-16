'use strict';

// Settings and the booking page's state (src/renderer/booking-settings.js): which 'booking' events redraw the page
// that is showing. The renderer module runs in a vm with its imports stubbed, as in renderer-platform.test.js.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function load() {
  const code = fs
    .readFileSync(path.join(__dirname, '../src/renderer/booking-settings.js'), 'utf8')
    .replace(/^import .*;\r?\n/gm, '')
    .replace(/^export /gm, '');
  const context = vm.createContext({
    t: (key) => key,
    translate: (_locale, key) => key,
    getLanguage: () => 'en',
    getLocale: () => 'en',
    icons: {},
    esc: (v) => String(v),
    safeBookingUrl: (url) => (typeof url === 'string' && url.startsWith('https://') ? url : null),
    bookingFooterHtml: (_account, data) => `footer ${data.bookingLocale || 'en'} ${data.bookingUrl}`,
    signaturePreview: (html) => html
  });
  vm.runInContext(`${code}\nglobalThis.bookingPages = bookingPages;`, context);
  return context.bookingPages;
}

const OFF = { signedIn: true, status: 'off', url: null, settings: { locale: 'en' }, publishing: null, today: 1 };
const ON = { ...OFF, status: 'on', url: 'https://meet.example/demo-host' };

// A settings window showing `view`; a redraw draws the rows that view has, as settings.js does.
function settings(view) {
  const bookingPages = load();
  const shown = { view, redraws: 0 };
  const draw = () => {
    if (shown.view === 'main') pages.rowHtml();
    if (shown.view === 'signatures') pages.footerRow({ bookingFooter: true });
    if (shown.view === 'account') pages.accountFooterRow({ id: 'a1' }, { bookingFooter: true });
  };
  const pages = bookingPages({
    row: (r) => r,
    go() {},
    back() {},
    promptDialog() {},
    current: () => shown.view,
    rerender: () => {
      shown.redraws++;
      draw();
    }
  });
  pages.onState(OFF);
  draw();
  shown.redraws = 0;
  return { pages, shown };
}

for (const view of ['main', 'signatures', 'account']) {
  test(`${view}: a booking event redraws when the link or the page's language changes, and only then`, () => {
    const { pages, shown } = settings(view);
    // A publish or a new day changes nothing these rows show.
    pages.onState({ ...OFF, publishing: { unchecked: true }, today: 2 });
    assert.equal(shown.redraws, 0);
    // The page turned on: the footer gets its link.
    pages.onState(ON);
    assert.equal(shown.redraws, 1);
    pages.onState({ ...ON, today: 3 });
    assert.equal(shown.redraws, 1);
    // The page's language is the footer's language.
    pages.onState({ ...ON, settings: { locale: 'nl' } });
    assert.equal(shown.redraws, 2);
    // Turned off: the link goes.
    pages.onState(OFF);
    assert.equal(shown.redraws, 3);
  });
}

test('a page without the booking rows is not redrawn by booking events', () => {
  const { pages, shown } = settings('signatures');
  shown.view = 'aliases';
  pages.onState(ON);
  assert.equal(shown.redraws, 0);
});
