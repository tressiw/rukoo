'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');
const { tempDir } = require('./tmp');
const vm = require('vm');
const i18n = require('../src/i18n');
const en = require('../src/i18n/en');
const nl = require('../src/i18n/nl');
const { Engine } = require('../src/main/engine');

test.beforeEach(() => i18n.setLanguage('en'));

const forms = (value) => typeof value === 'string' ? [value] : Object.values(value);
const parameters = (text) => [...new Set([...text.matchAll(/\{(\w+)\}/g)].map((match) => match[1]))].sort();

test('registered catalogs have all keys, context, plural forms, and matching named parameters', () => {
  for (const language of i18n.languages) {
    const catalog = require(`../src/i18n/${language.code}`);
    assert.deepEqual(Object.keys(catalog).sort(), Object.keys(en).sort(), language.code);
    for (const [key, entry] of Object.entries(en)) {
      assert.ok(entry.description.length > 15, `Missing context: ${key}`);
      const sourceForms = forms(entry.message);
      for (const text of sourceForms) {
        assert.ok(typeof text === 'string' && text.length, `Empty source: ${key}`);
        assert.deepEqual(parameters(text), parameters(sourceForms[0]), `Source parameters: ${key}`);
      }
      const translated = language.code === 'en' ? catalog[key].message : catalog[key];
      if (typeof entry.message === 'object') {
        assert.equal(typeof translated, 'object', `Missing plural forms: ${language.code}.${key}`);
        assert.ok(translated.other, `Missing plural fallback: ${language.code}.${key}`);
        for (const category of Object.keys(entry.message)) assert.ok(translated[category], `Missing ${category}: ${language.code}.${key}`);
      }
      for (const text of forms(translated)) {
        assert.ok(typeof text === 'string' && text.length, `Empty translation: ${language.code}.${key}`);
        assert.deepEqual(parameters(text), parameters(sourceForms[0]), `Parameters: ${language.code}.${key}`);
      }
    }
  }
});

// macOS shows Info.plist usage descriptions in its own questions, such as the one about the local network. English
// comes from build.mac.extendInfo in package.json. Every other language has <code>.lproj/InfoPlist.strings with the
// same keys, which build.mac.extraResources copies into the app.
test('macOS permission texts are translated in an InfoPlist.strings that the mac build packages', () => {
  const { mac } = require('../package.json').build;
  const english = Object.fromEntries(Object.entries(mac.extendInfo).filter(([key]) => key.endsWith('UsageDescription')));
  assert.ok(english.NSLocalNetworkUsageDescription);
  // A strings file holds "key" = "value"; entries and /* */ comments, nothing else.
  const entry = /"((?:[^"\\]|\\.)*)"\s*=\s*"((?:[^"\\]|\\.)*)"\s*;/g;
  const unescape = (text) => text.replace(/\\(["\\])/g, '$1');
  for (const { code } of i18n.languages.filter((language) => language.code !== 'en')) {
    const folder = `build/mac/${code}.lproj`;
    assert.ok((mac.extraResources || []).some(({ from, to }) => from === folder && to === `${code}.lproj`), `${folder} is not packaged`);
    const source = fs.readFileSync(path.join(__dirname, '..', folder, 'InfoPlist.strings'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
    assert.equal(source.replace(entry, '').trim(), '', `${folder}/InfoPlist.strings has text outside its entries`);
    const strings = Object.fromEntries([...source.matchAll(entry)].map(([, key, value]) => [unescape(key), unescape(value)]));
    assert.deepEqual(Object.keys(strings).sort(), Object.keys(english).sort(), folder);
    for (const [key, text] of Object.entries(strings)) assert.ok(text.length > 20 && text !== english[key], `Untranslated: ${code}.${key}`);
  }
});

test('all source translation references and persistent DOM labels exist in the English catalog', () => {
  for (const directory of ['main', 'renderer']) {
    // Subfolders too: the agent panel lives in renderer/agent and its main side in main/agents.
    for (const filename of fs.readdirSync(path.join(__dirname, '../src', directory), { recursive: true })) {
      if (!/\.(js|html)$/.test(filename)) continue;
      const source = fs.readFileSync(path.join(__dirname, '../src', directory, filename), 'utf8');
      // t('key'), translate(language, 'key') with one level of calls in the language, and data-i18n attributes.
      const references = /\bt\(['"]([^'"]+)['"]|\btranslate\((?:[^,()\n]|\([^()\n]*\))+,\s*['"]([^'"]+)['"]|data-i18n(?:-[\w-]+)?="([\w.]+)"/g;
      for (const match of source.matchAll(references)) {
        const key = match[1] || match[2] || match[3];
        assert.ok(en[key], `Unknown key ${key} in ${directory}/${filename}`);
      }
    }
  }
});

test('English is the default; missing translations and unsupported preferences fall back to English', () => {
  assert.equal(i18n.getLanguage(), 'en');
  assert.equal(i18n.t('mailbox.folders.inbox'), 'Inbox');
  i18n.setLanguage('nl');
  assert.equal(i18n.t('mailbox.folders.inbox'), 'Postvak IN');
  const saved = nl['mailbox.folders.inbox'];
  delete nl['mailbox.folders.inbox'];
  try {
    assert.equal(i18n.t('mailbox.folders.inbox'), 'Inbox');
  } finally {
    nl['mailbox.folders.inbox'] = saved;
  }
  i18n.setLanguage('unsupported');
  assert.equal(i18n.getLanguage(), 'en');
  assert.throws(() => i18n.t('missing.key'), /Unknown translation key/);
});

test('interpolation keeps user text literal and plural rules handle zero, one, and many', () => {
  assert.equal(i18n.t('setup.account.added', { email: '<b>{count}</b>' }), '<b>{count}</b> has been added');
  assert.throws(() => i18n.t('setup.account.added'), /Missing translation parameter email/);
  assert.equal(i18n.t('native.notifications.newCount', { count: 0 }), '0 new emails');
  assert.equal(i18n.t('native.notifications.newCount', { count: 1 }), '1 new email');
  assert.equal(i18n.t('native.notifications.newCount', { count: 3 }), '3 new emails');
  i18n.setLanguage('nl');
  assert.equal(i18n.t('settings.aliases.count', { count: 1 }), '1 alias');
  assert.equal(i18n.t('settings.aliases.count', { count: 3 }), '3 aliassen');
});

test('translate gives one language whatever the interface shows, with its plural rules and the English fallback', () => {
  assert.equal(i18n.translate('nl', 'mailbox.folders.inbox'), 'Postvak IN');
  assert.equal(i18n.getLanguage(), 'en');
  assert.equal(i18n.t('mailbox.folders.inbox'), 'Inbox');
  i18n.setLanguage('nl');
  assert.equal(i18n.translate('en', 'mailbox.folders.inbox'), 'Inbox');
  assert.equal(i18n.translate('en', 'native.notifications.newCount', { count: 1 }), '1 new email');
  assert.equal(i18n.translate('nl', 'settings.aliases.count', { count: 3 }), '3 aliassen');
  assert.equal(i18n.translate('unsupported', 'mailbox.folders.inbox'), 'Inbox');
  assert.equal(i18n.translate(null, 'setup.account.added', { email: '{count}' }), '{count} has been added');
  const saved = nl['mailbox.folders.inbox'];
  delete nl['mailbox.folders.inbox'];
  try {
    assert.equal(i18n.translate('nl', 'mailbox.folders.inbox'), 'Inbox');
  } finally {
    nl['mailbox.folders.inbox'] = saved;
  }
  assert.throws(() => i18n.translate('nl', 'missing.key'), /Unknown translation key/);
});

test('the same catalogs load in the sandboxed renderer and update document language', () => {
  const document = { documentElement: {} };
  const context = vm.createContext({ document, Intl });
  for (const filename of ['en.js', 'nl.js', 'index.js']) {
    vm.runInContext(fs.readFileSync(path.join(__dirname, '../src/i18n', filename), 'utf8'), context);
  }
  assert.equal(context.RukooI18n.t('composer.actions.send'), 'Send');
  context.RukooI18n.setLanguage('nl');
  assert.equal(context.RukooI18n.t('composer.actions.send'), 'Verzenden');
  assert.equal(context.RukooI18n.translate('en', 'composer.actions.send'), 'Send');
  assert.equal(document.documentElement.lang, 'nl');
  assert.equal(document.documentElement.dir, 'ltr');
});

test('language persists across restarts, migrates old profiles to English, and leaves folder paths intact', async () => {
  const dataDir = tempDir('rukoo-i18n-');
  const engine = new Engine({ dataDir }).init();
  try {
    assert.equal(engine.settings.language, 'en');
    const account = await engine.addAccount({ type: 'demo' });
    await engine.syncAccount(account.id);
    const before = engine.state().accounts[0].folders;
    assert.equal(before.find((folder) => folder.role === 'inbox').name, 'Inbox');
    engine.updateSettings({ language: 'nl' });
    const after = engine.state().accounts[0].folders;
    assert.deepEqual(after.map((folder) => folder.path), before.map((folder) => folder.path));
    assert.equal(after.find((folder) => folder.role === 'inbox').name, 'Postvak IN');
    await engine.close();
    const restarted = new Engine({ dataDir }).init();
    assert.equal(restarted.settings.language, 'nl');
    restarted.updateSettings({ language: 'unknown' });
    assert.equal(restarted.settings.language, 'en');
    await restarted.close();
    fs.writeFileSync(path.join(dataDir, 'settings.json'), JSON.stringify({ theme: 'dark' }));
    const legacy = new Engine({ dataDir }).init();
    assert.equal(legacy.settings.language, 'en');
    assert.equal(legacy.settings.theme, 'dark');
    await legacy.close();
  } finally {
    await engine.close();
    fs.rmSync(dataDir, { recursive: true, force: true });
  }
});

test('date and size formatting follow the selected interface language', () => {
  const source = fs.readFileSync(path.join(__dirname, '../src/renderer/ui.js'), 'utf8')
    .split('// ---------- previews ----------')[0]
    .replace(/^import .*;\r?\n/gm, '')
    .replace(/export /g, '');
  const context = vm.createContext({ ...i18n, Intl, window: {}, document: {} });
  vm.runInContext(`${source}\nglobalThis.format = { groupLabel, longDate, readerDate, fileSize };`, context);
  const date = new Date(2026, 9, 7, 14, 45).getTime();
  assert.equal(context.format.groupLabel(date, date), 'Today');
  assert.match(context.format.longDate(date), /October/);
  assert.equal(context.format.fileSize(1.5 * 1024 * 1024), '1.5 MB');
  i18n.setLanguage('nl');
  assert.equal(context.format.groupLabel(date, date), 'Vandaag');
  assert.match(context.format.longDate(date), /oktober/);
  assert.equal(context.format.fileSize(1.5 * 1024 * 1024), '1,5 MB');
});

test('an email whose date could not be read shows without one, instead of throwing while the list draws', () => {
  i18n.setLanguage('en');
  const source = fs.readFileSync(path.join(__dirname, '../src/renderer/ui.js'), 'utf8')
    .split('// ---------- previews ----------')[0]
    .replace(/^import .*;\r?\n/gm, '')
    .replace(/export /g, '');
  const context = vm.createContext({ ...i18n, Intl, window: {}, document: {} });
  vm.runInContext(`${source}\nglobalThis.format = { groupLabel, listTime, longDate, readerDate, numericDate, hhmm, quoteDate };`, context);
  for (const bad of [NaN, null, undefined, 'Mon, 32 Foo 2026']) {
    for (const [name, fn] of Object.entries(context.format)) {
      assert.equal(fn(bad), '', `${name}(${String(bad)})`);
    }
  }
  const date = new Date(2026, 9, 7, 14, 45).getTime();
  assert.equal(context.format.listTime(date, date), '14:45', 'a real date still shows');
});
