// Behavior checks for the pill and Settings against their mocks: what a user does, and what the app would receive.
// Development only. Runs the repository's Electron through Playwright, off-screen and without taking focus.
//   node voice/ui/dev/check.js
'use strict';

const { _electron: electron } = require('@playwright/test');
const assert = require('node:assert/strict');
const path = require('path');
const { start } = require('./serve.js');

const checks = [];
const check = (name, run) => checks.push({ name, run });

// ---------- Settings ----------

check('a switch saves at once and shows the new state', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&theme=dark');
  const row = page.locator('[data-a="toggle:dictation.readyChime"]');
  assert.equal(await row.getAttribute('aria-checked'), 'false');
  await row.click();
  assert.equal(await row.getAttribute('aria-checked'), 'true');
  assert.equal((await mockState()).config.dictation.readyChime, true);
});

check('a key list greys out the other keys in use and saves the choice', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&theme=dark');
  await page.click('[data-a="choose:dictation.hotkey"]');
  assert.equal(await page.locator('.radio-row', { hasText: 'Right Ctrl' }).isDisabled(), true, 'the rewrite key is taken');
  await page.locator('.radio-row', { hasText: 'F13' }).click();
  assert.equal((await mockState()).config.dictation.hotkey, 'f13');
  await page.waitForSelector('[data-a="choose:dictation.hotkey"] .value:text("F13")');
  assert.match(await page.locator('[data-section="status"]').innerText(), /Hold F13 to dictate/);
});

check('Escape goes back one page, and the back button too', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=dictionary');
  assert.equal(await page.locator('h1').innerText(), 'Dictionary');
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('h1').innerText(), 'Rukoo Voice');
  await page.click('[data-a="go:rukoo"]');
  await page.click('[data-a="back"]');
  assert.equal(await page.locator('h1').innerText(), 'Rukoo Voice');
});

check('text being typed survives the download progress redrawing its card', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=transcription&scenario=fresh');
  await page.click('[data-a="model:orukeet:download"]');
  const minutes = page.locator('[data-field="local.idleMinutes"]');
  await minutes.click();
  await minutes.press('Control+A');
  for (const key of ['4', '5']) {
    await page.keyboard.type(key);
    await page.waitForTimeout(450);
  }
  // The card redrew at least twice in the meantime (the mock ticks every 300 ms).
  assert.equal(await minutes.inputValue(), '45');
  assert.equal(await page.evaluate(() => document.activeElement?.dataset.field), 'local.idleMinutes');
});

check('changing the provider clears the key and asks for consent again', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=meetingTranscription&scenario=cloud');
  assert.equal(await page.locator('.key-saved').count(), 1, 'Mistral has a saved key');
  const consent = page.locator('[data-field="meetings.consent"]');
  assert.equal(await consent.isChecked(), false);
  await consent.check();
  await page.selectOption('[data-field="meetings.provider"]', 'openai');
  assert.equal(await page.locator('.key-saved').count(), 0, "Mistral's key isn't offered for OpenAI");
  assert.equal(await page.inputValue('[data-field="meetings.endpoint"]'), 'https://api.openai.com/v1/audio/transcriptions');
  assert.equal(await page.inputValue('[data-field="meetings.model"]'), '');
  assert.equal(await page.locator('[data-field="meetings.consent"]').isChecked(), false, 'consent was for another provider');
  assert.match(await page.locator('.consent').innerText(), /api\.openai\.com/);
  await page.selectOption('[data-field="meetings.provider"]', 'orukeet');
  assert.equal(await page.locator('.consent').count(), 0, 'Orukeet keeps the audio here');
  assert.equal((await mockState()).config.meetings.provider, 'mistral', 'nothing saved without Save');
});

check('consent is saved per provider and host, and the top page stops waiting', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=meetingTranscription&scenario=cloud');
  await page.locator('[data-field="meetings.consent"]').check();
  await page.click('[data-a="save:meetings"]');
  await page.waitForSelector('.form-status.ok');
  assert.deepEqual((await mockState()).config.meetings.consent, { provider: 'mistral', host: 'api.mistral.ai', mode: 'windows' });
  const save = await page.evaluate(() => window.__lastSave);
  assert.equal(save.apiKey, null, 'the saved key is kept, never sent back');
  await page.fill('[data-field="meetings.endpoint"]', 'https://eu.api.mistral.ai/v1/audio/transcriptions');
  await page.locator('[data-field="meetings.endpoint"]').blur();
  assert.equal(await page.locator('[data-field="meetings.consent"]').isChecked(), false, 'another host asks again');
  await page.keyboard.press('Escape');
  assert.doesNotMatch(await page.locator('[data-section="meetings"]').innerText(), /Waiting for your consent/);
});

check('Save refuses a cloud provider without a model, with Ownkey\'s message', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=transcription');
  await page.selectOption('[data-field="transcription.provider"]', 'mistral');
  await page.click('[data-a="save:transcription"]');
  assert.match(await page.locator('.form-status').innerText(), /Choose an audio endpoint and model before saving\./);
});

check('a model list that arrives after the endpoint changed is dropped', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=rewriting');
  await page.click('[data-a="refresh:rewriting"]');
  await page.fill('[data-field="rewriting.endpoint"]', 'http://localhost:9999/v1/chat/completions');
  await page.waitForTimeout(1000);
  assert.equal(await page.locator('[data-form="rewriting"] .field-help').count(), 0);
  assert.equal(await page.locator('#models-rewriting option').count(), 0);
});

check('the dictionary follows Ownkey\'s rules', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=dictionary');
  await page.fill('[data-field="dict.word"]', '  rukoo  ');
  await page.press('[data-field="dict.word"]', 'Enter');
  const words = (await mockState()).config.dictionary.vocabulary;
  assert.deepEqual(words.filter((w) => w.toLowerCase() === 'rukoo'), ['rukoo'], 'replaced case-insensitively, not duplicated');
  await page.click('[data-a="dict-mode"]');
  await page.fill('[data-field="dict.from"]', 'Same');
  await page.fill('[data-field="dict.to"]', 'Same');
  await page.click('[data-a="dict-add"]');
  assert.match(await page.locator('[role="alert"]').innerText(), /The two spellings are identical\./);
  await page.click('[data-a^="dict-remove:fix:rocco"]');
  assert.equal((await mockState()).config.dictionary.corrections.some((c) => c.from === 'rocco'), false);
});

check('a model in Ownkey\'s folder offers no Remove download', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=transcription');
  assert.equal(await page.locator('[data-a="model:orukeet:remove"]').count(), 0);
  assert.match(await page.locator('.model-card').innerText(), /Shared with Ownkey\. Remove it in Ownkey\./);
});

check('Storage opens the recordings folder and offers no Browse that 0.1 would refuse', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=storage');
  assert.equal(await page.locator('[data-a="open:recordings"]').count(), 1);
  assert.equal(await page.locator('[data-a="folder:recordings"]').count(), 0);
  // The mock answers as the engine does.
  const refused = await page.evaluate(async () => {
    const { invoke } = await import('./lib/bridge.js');
    return invoke('settings_choose_folder', { purpose: 'recordings' }).then(
      () => null,
      (error) => error?.code
    );
  });
  assert.equal(refused, 'invalid_value');
});

check('a Settings state that isn\'t newer than the shown one is dropped', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&theme=dark');
  const row = page.locator('[data-a="toggle:dictation.readyChime"]');
  assert.equal(await row.getAttribute('aria-checked'), 'false');
  const shown = await mockState();
  const stale = structuredClone(shown);
  stale.config.dictation.readyChime = true;
  await page.evaluate((next) => window.__settingsMockSend(next), stale);
  await page.waitForTimeout(150);
  assert.equal(await row.getAttribute('aria-checked'), 'false', 'same seq: built before the shown state');
  await page.evaluate((next) => window.__settingsMockSend({ ...next, seq: next.seq + 1 }), stale);
  await page.waitForSelector('[data-a="toggle:dictation.readyChime"][aria-checked="true"]');
});

check('a change whose event arrives before the first settings_state answer stays on screen', async ({ open, page, mockState }) => {
  // The mock's first answer is the state from before the change, with the lower seq.
  await open('settings.html?mock=1&lang=en&theme=dark&eventFirst=1');
  await page.waitForTimeout(300);
  const app = await mockState();
  assert.equal(app.config.dictation.readyChime, true);
  assert.equal(await page.locator('[data-a="toggle:dictation.readyChime"]').getAttribute('aria-checked'), 'true', 'the older answer came back');
});

check('a development copy can\'t import or start at login', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&scenario=dev');
  assert.equal(await page.locator('[data-a="toggle:dictation.startAtLogin"]').isDisabled(), true);
  await page.click('[data-a="go:import"]');
  assert.equal(await page.locator('[data-a="import"]').isDisabled(), true);
  assert.match(await page.locator('.page-actions').innerText(), /--import-ownkey <copy>/);
});

check('Screen recording shows for a Voice that has it, and saves the choice', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en');
  assert.equal(await page.locator('[data-a^="seg:meetings.screen:"]').count(), 0, 'no screen feature, no row');
  await open('settings.html?mock=1&lang=en&page=storage');
  assert.doesNotMatch(await page.locator('.page').innerText(), /drops the video first/);
  await open('settings.html?mock=1&lang=en&features=screen');
  const row = page.locator('.row', { has: page.locator('[data-a^="seg:meetings.screen:"]') });
  assert.match(await row.innerText(), /Screen recording/);
  assert.equal(await row.locator('.note').count(), 0, 'this computer records everything');
  await page.click('[data-a="seg:meetings.screen:video"]');
  assert.equal((await mockState()).config.meetings.screen, 'video');
  await page.waitForSelector('[data-a="seg:meetings.screen:video"][aria-selected="true"]');
  await open('settings.html?mock=1&lang=en&page=storage&features=screen');
  assert.match(await page.locator('.page').innerText(), /When the disk fills up, Voice drops the video first, then the frames, and never the audio\./);
});

check('Screen recording says what this computer can\'t record, and is off where Windows can\'t capture', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&noEncoder=1&noOcr=1');
  const notes = page.locator('.row .note');
  assert.deepEqual(await notes.allInnerTexts(), [
    'This computer has no hardware video encoder, so Video records frames only.',
    'Windows has no text recognition language installed, so frames are saved without their text.'
  ]);
  await open('settings.html?mock=1&lang=nl&noEncoder=1&noOcr=1');
  assert.deepEqual(await notes.allInnerTexts(), [
    'Deze computer heeft geen hardware-video-encoder, dus Video neemt alleen beelden op.',
    'Windows heeft geen taal voor tekstherkenning, dus beelden worden zonder tekst bewaard.'
  ]);
  await open('settings.html?mock=1&lang=en&screenUnavailable=1');
  assert.deepEqual(await notes.allInnerTexts(), ['Screen recording needs Windows 10 version 2004 or later.']);
  assert.equal(await page.locator('[data-a="seg:meetings.screen:frames"]').isDisabled(), true);
  await page.locator('[data-a="seg:meetings.screen:frames"]').click({ force: true });
  assert.equal((await mockState()).config.meetings.screen, 'off', 'a disabled choice saves nothing');
});

// ---------- speaker labels ----------

const SPEAKER_TEXT = {
  off: 'Off. Meetings label the whole call as one speaker.',
  platform: 'Speaker labels need Windows on an Intel or AMD PC, or an Apple Silicon Mac.',
  cpu: 'This processor lacks BMI2, which speaker labels need. Meetings label the whole call as one speaker.',
  runtime: 'The speaker label runtime is missing from this installation. Reinstall Rukoo Voice.',
  runtimeFailed: "The speaker label runtime couldn't start. Voice tries again after a restart.",
  model: 'Download the speaker model to label speakers.',
  modelDamaged: 'The speaker model is damaged. Remove it and download it again.',
  key: 'Transcription and labels wait for an API key for Mistral. Add one in Meeting transcription.',
  consent: 'Transcription and labels wait until you agree to stream meeting audio to Mistral.'
};

check('the Speaker labels row shows only with the feature, and says why labels are off', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=meetings');
  assert.equal(await page.locator('[data-a="go:speakers"]').count(), 0, 'no speakers feature, no row');
  await open('settings.html?mock=1&lang=en&page=meetings&speakers=ready');
  assert.match(await page.locator('[data-a="go:speakers"]').innerText(), /Speaker labels\s+On/);
  await open('settings.html?mock=1&lang=en&page=meetings&speakers=cpu');
  const text = await page.locator('[data-a="go:speakers"]').innerText();
  assert.match(text, /Unavailable/);
  assert.ok(text.includes(SPEAKER_TEXT.cpu), text);
  await open('settings.html?mock=1&lang=en&page=meetings&speakers=off');
  assert.match(await page.locator('[data-a="go:speakers"]').innerText(), /Speaker labels\s+Off/);
});

check('a Voice with screen recording and speaker labels shows both rows, each from its own feature', async ({ open, page }) => {
  const rows = async () => ({
    screen: await page.locator('[data-a^="seg:meetings.screen:"]').count(),
    speakers: await page.locator('[data-a="go:speakers"]').count()
  });
  await open('settings.html?mock=1&lang=en&page=meetings&features=screen,speakers');
  assert.deepEqual(await rows(), { screen: 3, speakers: 1 });
  assert.match(await page.locator('[data-a="go:speakers"]').innerText(), /Speaker labels/);
  await open('settings.html?mock=1&lang=en&page=meetings&features=screen');
  assert.deepEqual(await rows(), { screen: 3, speakers: 0 }, 'screen alone');
  await open('settings.html?mock=1&lang=en&page=meetings&features=speakers');
  assert.deepEqual(await rows(), { screen: 0, speakers: 1 }, 'speakers alone');
});

check('every reason labels are off shows its text on the Speaker labels page', async ({ open, page }) => {
  for (const [reason, text] of Object.entries(SPEAKER_TEXT)) {
    await open(`settings.html?mock=1&lang=en&page=speakers&speakers=${reason}`);
    const status = await page.locator('[data-section="status"]').innerText();
    assert.ok(status.includes(text), `${reason}: ${status}`);
    assert.match(status, reason === 'off' ? /\bOff\b/ : /Unavailable/, reason);
  }
  await open('settings.html?mock=1&lang=en&page=speakers&speakers=ready');
  assert.match(await page.locator('[data-section="status"]').innerText(), /Ready\. New meetings label each voice on the call\.[\s\S]*Nemotron on this computer/);
});

check('the switch stays usable without the model and applies to new meetings', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=speakers&speakers=model');
  const toggle = page.locator('[data-a="toggle:meetings.liveSpeakers"]');
  assert.equal(await toggle.isDisabled(), false);
  assert.match(await toggle.innerText(), /Applies to meetings you start from now on\./);
  await toggle.click();
  assert.equal((await mockState()).config.meetings.liveSpeakers, false);
  await page.waitForSelector('[data-section="status"] :text("Off. Meetings label the whole call as one speaker.")');
  // Downloading the model makes labels ready once the switch is on again.
  await toggle.click();
  await page.click('[data-a="model:nemotron:download"]');
  await page.waitForSelector('[data-section="status"] :text("Ready. New meetings label each voice on the call.")', { timeout: 10000 });
  assert.equal((await mockState()).models.nemotron.removable, true);
  // Nemotron has no loaded state, so its files never read "Not active" next to a ready status.
  assert.equal(await page.locator('[data-section="model"] .model-status').innerText(), 'Installed');
});

check('Error details shows why the runtime failed or the model is damaged', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=speakers&speakers=runtimeFailed');
  await page.click('[data-a="speakers-details"]');
  assert.match(await page.locator('.dialog .detail').innerText(), /nemo_speech_asr_c\.dll/);
  await page.keyboard.press('Escape');
  await open('settings.html?mock=1&lang=en&page=speakers&speakers=modelDamaged');
  await page.click('[data-a="speakers-details"]');
  assert.match(await page.locator('.dialog .detail').innerText(), /Checksum mismatch/);
  await page.keyboard.press('Escape');
  // A reason without an error text has no Error details.
  await open('settings.html?mock=1&lang=en&page=speakers&speakers=runtime');
  assert.equal(await page.locator('[data-a="speakers-details"]').count(), 0);
});

check('Mistral labels a streamed meeting, and a stopped meeting is called out', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=speakers&speakers=mistral');
  const status = await page.locator('[data-section="status"]').innerText();
  assert.match(status, /Ready\. Mistral labels the voices on the call/);
  await page.click('[data-a="go:meetingTranscription"]');
  assert.equal(await page.locator('h1').innerText(), 'Meeting transcription');
  await open('settings.html?mock=1&lang=en&page=speakers&speakers=stopped');
  assert.match(await page.locator('[data-section="status"]').innerText(), /Speaker labels stopped for the current meeting\. Recording and transcription go on\./);
});

check('removing the meeting key turns Mistral labels off and points to Meeting transcription', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=speakers&speakers=mistral');
  assert.match(await page.locator('[data-section="status"]').innerText(), /Ready\. Mistral labels/);
  await page.click('[data-a="go:meetingTranscription"]');
  await page.click('[data-a="remove-key:meetings"]');
  const state = await mockState();
  assert.equal(state.config.meetings.keySaved, false);
  assert.equal(state.config.meetings.consent?.mode, 'stream', 'the agreement stays');
  assert.equal(state.speakers.reason, 'key');
  await page.click('[data-a="back"]');
  const status = await page.locator('[data-section="status"]').innerText();
  assert.ok(status.includes(SPEAKER_TEXT.key), status);
  assert.match(status, /Unavailable/);
  await page.click('[data-a="back"]');
  const row = await page.locator('[data-a="go:speakers"]').innerText();
  assert.ok(row.includes(SPEAKER_TEXT.key), row);
});

check('a realtime model asks to stream, and a batch agreement doesn\'t cover it', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=meetingTranscription&scenario=cloud');
  assert.match(await page.locator('.consent').innerText(), /in short pieces/);
  await page.locator('[data-field="meetings.consent"]').check();
  await page.click('[data-a="save:meetings"]');
  await page.waitForSelector('.form-status.ok');
  assert.equal((await mockState()).config.meetings.consent.mode, 'windows');
  await page.fill('[data-field="meetings.model"]', 'voxtral-mini-transcribe-realtime-3');
  await page.locator('[data-field="meetings.model"]').blur();
  const consent = page.locator('.consent');
  assert.match(await consent.innerText(), /Voice streams the recorded tracks to Mistral/);
  assert.match(await consent.innerText(), /continuous stream, and clips of up to 15 seconds per voice/);
  assert.match(await consent.innerText(), /Stream meeting audio to Mistral at api\.mistral\.ai/);
  assert.equal(await page.locator('[data-field="meetings.consent"]').isChecked(), false, 'the batch agreement is for windows');
  await page.locator('[data-field="meetings.consent"]').check();
  await page.click('[data-a="save:meetings"]');
  await page.waitForSelector('.form-status.ok');
  assert.deepEqual((await mockState()).config.meetings.consent, { provider: 'mistral', host: 'api.mistral.ai', mode: 'stream' });
  // Dictation can't use a realtime model.
  const refused = await page.evaluate(async () => {
    const { invoke } = await import('./lib/bridge.js');
    const save = { target: 'transcription', provider: 'mistral', endpoint: 'https://api.mistral.ai/v1/audio/transcriptions', model: 'voxtral-mini-transcribe-realtime-3', apiKey: null };
    return invoke('settings_save_provider', { save }).then(
      () => null,
      (error) => error?.code
    );
  });
  assert.equal(refused, 'invalid_value');
});

check('only the meetings form offers realtime models', async ({ open, page }) => {
  const offered = async (form, provider = 'mistral') => {
    await open(`settings.html?mock=1&lang=en&page=${form === 'meetings' ? 'meetingTranscription' : form}`);
    await page.selectOption(`[data-field="${form}.provider"]`, provider);
    await page.click(`[data-a="refresh:${form}"]`);
    await page.waitForSelector(`[data-form="${form}"] .field-help:text-matches("models? found")`);
    return page.locator(`#models-${form} option`).evaluateAll((options) => options.map((o) => o.value));
  };
  const dictation = await offered('transcription');
  assert.ok(dictation.includes('voxtral-mini-latest'), dictation.join());
  assert.ok(!dictation.some((id) => id.startsWith('voxtral-mini-transcribe-realtime')), dictation.join());
  assert.ok(!(await offered('rewriting')).some((id) => id.startsWith('voxtral-mini-transcribe-realtime')));
  assert.ok((await offered('meetings')).includes('voxtral-mini-transcribe-realtime-3'));
  // The engine refuses the name from every provider, so a proxy that lists it doesn't get it offered for dictation.
  const proxied = await offered('transcription', 'custom');
  assert.ok(proxied.includes('voxtral-mini-latest'), proxied.join());
  assert.ok(!proxied.some((id) => id.startsWith('voxtral-mini-transcribe-realtime')), proxied.join());
});

check('Dutch is complete on every page', async ({ open, page }) => {
  for (const name of ['main', 'transcription', 'dictionary', 'fillers', 'rewriting', 'meetingTranscription', 'speakers', 'storage', 'rukoo', 'import']) {
    await open(`settings.html?mock=1&lang=nl&scenario=cloud&features=screen&speakers=cpu&page=${name}`);
    const text = await page.locator('.page').innerText();
    for (const english of ['Settings', 'Save', 'Transcription', 'Dictionary', 'Rewriting', 'Storage', 'Connection', 'Import from']) {
      assert.ok(!new RegExp(`\\b${english}\\b`).test(text), `"${english}" on the Dutch ${name} page`);
    }
  }
});

// ---------- Settings > Storage: bucket sync ----------

const ZERO_CODE = `RKV1-${Array(11).fill('00000').join('-')}`;
const blurFill = async (page, field, value) => {
  const input = page.locator(`[data-field="${field}"]`);
  await input.fill(value);
  await input.blur();
};
const protectedNow = (page) => page.evaluate(() => window.__settingsMockProtected());
const dialogText = (page) => page.locator('.dialog').innerText();
// Test connection pressed with the mouse, as a user does: the press takes the focus from the field being edited, which
// saves it, before the click lands. page.click() would focus the button first. Scrolling doesn't move the focus.
const press = async (page, selector) => {
  const target = page.locator(selector);
  await target.scrollIntoViewIfNeeded();
  const box = await target.boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
};
const pressTest = (page) => press(page, '[data-a="sync-test"]');

check('the sync card shows only when the engine lists sync in features', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=storage');
  assert.equal(await page.locator('[data-section="sync"]').count(), 0);
  await open('settings.html?mock=1&lang=en&page=storage&features=sync');
  assert.equal(await page.locator('[data-section="sync"]').count(), 1);
  assert.equal(await page.locator('[data-section="sync-connection"] .form').count(), 1, 'the form shows while sync is off');
});

check('a sync store that didn\'t open says nothing uploads and leaves the switch to turn off', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=unavailable');
  const card = page.locator('[data-section="sync"]');
  assert.equal(await card.count(), 1, 'the switch stays reachable');
  assert.equal(await page.locator('[data-section="sync-connection"]').count(), 0, 'nothing else of sync shows');
  const text = await card.innerText();
  assert.match(text, /Voice can't open its list of uploads, so nothing uploads\./);
  assert.match(text, /keeps meeting audio on this computer/);
  await page.click('[data-a="sync-details:unavailable"]');
  assert.match(await dialogText(page), /from a newer Voice/);
  await page.keyboard.press('Escape');
  const toggle = '[data-a="toggle:storage.sync.enabled"]';
  assert.equal(await page.locator(toggle).getAttribute('aria-checked'), 'true');
  await page.click(toggle);
  await page.waitForFunction(() => !document.querySelector('[data-section="sync"]'));
  assert.equal((await mockState()).config.storage.sync.enabled, false);
});

check('the switch waits for a passed Test connection, and a changed bucket pauses uploads', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=fresh');
  const toggle = '[data-a="toggle:storage.sync.enabled"]';
  assert.equal(await page.locator(toggle).isDisabled(), true);
  assert.match(await page.locator('[data-section="sync"]').innerText(), /Test the connection before you turn sync on\./);
  assert.equal(await page.locator('[data-a="sync-test"]').isDisabled(), true, 'no secret saved yet');
  for (const [name, value] of [
    ['endpoint', 'https://example.upcloudobjects.com'],
    ['region', 'europe-1'],
    ['bucket', 'my-meetings'],
    ['accessKeyId', 'example-access-key']
  ])
    await blurFill(page, `storage.sync.${name}`, value);
  await blurFill(page, 'storage.sync.secret', 'test-secret-1');
  await page.waitForSelector('[data-section="sync-connection"] .key-saved');
  assert.equal(await page.locator(toggle).isDisabled(), true, 'still untested');
  await page.click('[data-a="sync-test"]');
  await page.waitForSelector('[data-section="sync-connection"] .form-status.ok');
  assert.match(await page.locator('[data-section="sync-connection"] .form-status').innerText(), /Connected to the bucket/);
  await page.waitForSelector(`${toggle}:not([disabled])`);
  assert.match(await page.locator('[data-section="sync"]').innerText(), /newest first/, 'says what turning it on uploads');
  await page.click(toggle);
  await page.waitForSelector(`${toggle}[aria-checked="true"]`);
  assert.equal((await mockState()).config.storage.sync.enabled, true);
  await page.waitForSelector('[data-section="sync"] .sync-status .status');
  // The switch stays on, so retention keeps holding what hasn't reached the bucket; uploads wait for a test.
  await blurFill(page, 'storage.sync.bucket', 'other-bucket');
  await page.waitForFunction(() => /Uploads paused\. Test the connection to go on\./.test(document.querySelector('[data-section="sync"]').innerText));
  assert.equal(await page.locator(toggle).getAttribute('aria-checked'), 'true');
  assert.equal(await page.locator(toggle).isDisabled(), false, 'the user can still turn it off');
  assert.equal(await page.locator('[data-section="sync-connection"] .form-status.ok').count(), 0, 'the old test no longer counts');
  assert.equal((await mockState()).config.storage.sync.enabled, true);
  // A passed test lets uploads go on without touching the switch.
  await page.click('[data-a="sync-test"]');
  await page.waitForSelector('[data-section="sync-connection"] .form-status.ok');
  await page.waitForFunction(() => !/Uploads paused/.test(document.querySelector('[data-section="sync"]').innerText));
  assert.notEqual((await mockState()).sync.state, 'paused');
});

check('a new secret while uploads fail pauses them and keeps the switch on', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=failing');
  await page.click('[data-a="sync-secret-replace"]');
  await blurFill(page, 'storage.sync.secret', 'test-secret-7');
  await page.waitForSelector('[data-section="sync-connection"] .key-saved');
  await page.waitForFunction(() => /Uploads paused/.test(document.querySelector('[data-section="sync"]').innerText));
  const text = await page.locator('[data-section="sync"]').innerText();
  assert.match(text, /Uploads paused\. Test the connection to go on\.[\s\S]*41 files waiting[\s\S]*as long as 10 GB stay free/);
  const sync = (await mockState()).config.storage.sync;
  assert.deepEqual([sync.enabled, sync.tested, sync.secretSaved], [true, false, true]);
  assert.equal(await page.locator('[data-a="toggle:storage.sync.enabled"]').getAttribute('aria-checked'), 'true');
});

check('a switch the app refuses with needs_test rolls back and says why', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=tested');
  // The app no longer counts the test, and the page hasn't heard yet.
  await page.evaluate(() => window.__settingsMockEdit((s) => (s.config.storage.sync.tested = false), { emit: false }));
  await page.click('[data-a="toggle:storage.sync.enabled"]');
  await page.waitForSelector('.dialog');
  assert.match(await dialogText(page), /Test the connection before you turn sync on\./);
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('[data-a="toggle:storage.sync.enabled"]').getAttribute('aria-checked'), 'false');
});

check('the bucket secret goes to settings_sync_save_secret on blur and never through settings_set', async ({ open, page, calls }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=fresh');
  await blurFill(page, 'storage.sync.secret', 'test-secret-2');
  await page.waitForSelector('[data-section="sync-connection"] .key-saved');
  const log = await calls();
  assert.ok(log.some(([command, args]) => command === 'settings_sync_save_secret' && args.secret === 'test-secret-2'));
  assert.ok(!log.some(([command, args]) => command === 'settings_set' && JSON.stringify(args).includes('test-secret-2')), 'settings_set never sees it');
  assert.ok(!log.some(([command, args]) => command === 'settings_set' && args.key === 'storage.sync.secret'));
  assert.ok(!(await page.content()).includes('test-secret-2'), 'the page keeps no copy');
  await page.click('[data-a="sync-secret-replace"]');
  assert.equal(await page.inputValue('[data-field="storage.sync.secret"]'), '', 'Replace shows an empty field');
  // Escape leaves the page and so the field: the secret is saved as on any blur, and the page keeps nothing.
  await page.fill('[data-field="storage.sync.secret"]', 'test-secret-3');
  await page.keyboard.press('Escape');
  await page.click('[data-a="go:storage"]');
  await page.waitForSelector('[data-section="sync-connection"] .key-saved');
  await page.click('[data-a="sync-secret-replace"]');
  assert.equal(await page.inputValue('[data-field="storage.sync.secret"]'), '');
  const sent = (await calls()).filter(([, args]) => JSON.stringify(args).includes('test-secret-3'));
  assert.deepEqual(
    sent.map(([command]) => command),
    ['settings_sync_save_secret'],
    'only settings_sync_save_secret sees it'
  );
  assert.ok(!(await page.content()).includes('test-secret-3'));
});

check('a test still running when the secret is replaced never saves it half-typed', async ({ open, page, calls }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=on');
  // Its answer is about the old secret. Redrawing the card for it would take the focused field out of the page, which
  // Chromium commits as on blur: "test-" would be saved and the rest of the typing lost.
  await page.click('[data-a="sync-test"]');
  await page.click('[data-a="sync-secret-replace"]');
  await page.click('[data-field="storage.sync.secret"]');
  await page.keyboard.type('test-');
  await page.waitForTimeout(900); // the mock answers Test connection after 600 ms
  await page.keyboard.type('secret-5');
  assert.equal(await page.inputValue('[data-field="storage.sync.secret"]'), 'test-secret-5', 'the field kept its text and focus');
  assert.equal(await page.locator('[data-section="sync-connection"] .form-status.ok').count(), 0, 'the old secret\'s test is dropped');
  const saves = () => calls().then((log) => log.filter(([command]) => command === 'settings_sync_save_secret').map(([, args]) => args.secret));
  assert.deepEqual(await saves(), [], 'nothing saved before the field loses focus');
  await page.locator('[data-field="storage.sync.secret"]').blur();
  await page.waitForSelector('[data-section="sync-connection"] .key-saved');
  assert.deepEqual(await saves(), ['test-secret-5']);
});

check('Remove deletes the bucket secret and pauses uploads', async ({ open, page, calls, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=on');
  await page.click('[data-a="sync-secret-remove"]');
  await page.waitForSelector('[data-field="storage.sync.secret"]');
  assert.ok((await calls()).some(([command, args]) => command === 'settings_remove_key' && args.target === 'sync'));
  const sync = (await mockState()).config.storage.sync;
  assert.deepEqual([sync.secretSaved, sync.tested, sync.enabled], [false, false, true]);
  await page.waitForFunction(() => /Uploads paused/.test(document.querySelector('[data-section="sync"]').innerText));
  assert.equal(await page.locator('[data-a="sync-test"]').isDisabled(), true);
});

check('a failed Test connection says why, with Error details', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=tested');
  await blurFill(page, 'storage.sync.bucket', 'fail');
  await page.click('[data-a="sync-test"]');
  await page.waitForSelector('[data-section="sync-connection"] .form-status.bad');
  assert.match(await page.locator('[data-section="sync-connection"] .form-status').innerText(), /There's no bucket with this name at this endpoint\./);
  await page.click('[data-a="sync-details:test"]');
  assert.match(await dialogText(page), /NoSuchBucket/);
  await page.keyboard.press('Escape');
  // A bucket that holds another key's meetings says so.
  await blurFill(page, 'storage.sync.bucket', 'shared');
  await page.click('[data-a="sync-test"]');
  await page.waitForSelector('[data-section="sync-connection"] .form-status.ok');
  assert.match(await page.locator('[data-section="sync-connection"]').innerText(), /another encryption key/);
});

check('one click on Test connection right after typing the secret saves it, then tests', async ({ open, page, calls }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=fresh');
  for (const [name, value] of [
    ['endpoint', 'https://example.upcloudobjects.com'],
    ['bucket', 'my-meetings'],
    ['accessKeyId', 'example-access-key']
  ])
    await blurFill(page, `storage.sync.${name}`, value);
  const test = page.locator('[data-a="sync-test"]');
  assert.equal(await test.isDisabled(), true, 'nothing to test without a secret');
  await page.click('[data-field="storage.sync.secret"]');
  await page.keyboard.type('test-secret-4');
  assert.equal(await test.isDisabled(), false, 'a typed secret can be tested');
  assert.equal(await page.inputValue('[data-field="storage.sync.secret"]'), 'test-secret-4', 'typing goes on after the button turns on');
  await pressTest(page);
  await page.waitForSelector('[data-section="sync-connection"] .form-status.ok');
  const order = (await calls()).map(([command]) => command).filter((command) => command.startsWith('settings_sync_'));
  assert.deepEqual(order, ['settings_sync_save_secret', 'settings_sync_test'], 'saved first, then tested, from one click');
  // Pasted in one go, with a state from the app redrawing the page while it sits in the field.
  await open('settings.html?mock=1&lang=en&page=storage&sync=fresh');
  await page.fill('[data-field="storage.sync.secret"]', 'test-secret-6');
  await page.evaluate(() => window.__settingsMockEdit((s) => (s.storage.usedBytes += 1_000_000)));
  await page.waitForTimeout(100);
  assert.equal(await page.inputValue('[data-field="storage.sync.secret"]'), 'test-secret-6');
  assert.equal(await test.isDisabled(), false);
  await pressTest(page);
  await page.waitForSelector('[data-section="sync-connection"] .form-status.bad');
  const sent = (await calls()).filter(([command]) => command.startsWith('settings_sync_'));
  assert.deepEqual(
    sent.map(([command, args]) => (command === 'settings_sync_save_secret' ? `${command}:${args.secret}` : command)),
    ['settings_sync_save_secret:test-secret-6', 'settings_sync_test']
  );
  // Typing and emptying the field again leaves nothing to test.
  await open('settings.html?mock=1&lang=en&page=storage&sync=fresh');
  await page.click('[data-field="storage.sync.secret"]');
  await page.keyboard.type('x');
  await page.keyboard.press('Backspace');
  assert.equal(await test.isDisabled(), true);
});

check('after a failed test, fixing a field and clicking Test once tests again', async ({ open, page, calls }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=tested');
  await blurFill(page, 'storage.sync.bucket', 'fail');
  await page.click('[data-a="sync-test"]');
  await page.waitForSelector('[data-section="sync-connection"] .form-status.bad');
  // The press takes the focus from the field, which saves it and clears the old result; the click still counts.
  await page.click('[data-field="storage.sync.bucket"]');
  await page.keyboard.press('End');
  await page.keyboard.type('-fixed');
  await pressTest(page);
  await page.waitForSelector('[data-section="sync-connection"] .form-status.ok');
  const log =(await calls()).filter(([command, args]) => command === 'settings_sync_test' || args.key === 'storage.sync.bucket');
  assert.deepEqual(
    log.map(([command, args]) => (command === 'settings_set' ? args.value : command)),
    ['fail', 'settings_sync_test', 'fail-fixed', 'settings_sync_test'],
    'the fixed name saved, then tested'
  );
});

check('an http endpoint is refused with its own text and stays in the field', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=fresh');
  await blurFill(page, 'storage.sync.endpoint', 'http://example.com');
  await page.waitForSelector('.dialog');
  assert.match(await dialogText(page), /It starts with https:\/\//);
  await page.keyboard.press('Escape');
  assert.equal(await page.inputValue('[data-field="storage.sync.endpoint"]'), 'http://example.com');
});

// Test connection and the switch both use the saved fields, so neither may run while a field shows a value the app
// refused: the page would say "Connected to the bucket", or sync would start, for an endpoint the form doesn't show.
check('a refused connection value blocks Test connection and the switch until a value saves', async ({ open, page, calls, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=tested');
  const endpoint = '[data-field="storage.sync.endpoint"]';
  const toggle = '[data-a="toggle:storage.sync.enabled"]';
  const connection = page.locator('[data-section="sync-connection"]');
  const sent = async (match) => (await calls()).filter(match).length;
  const tests = () => sent(([command]) => command === 'settings_sync_test');
  const switchesOn = () => sent(([command, args]) => command === 'settings_set' && args.key === 'storage.sync.enabled' && args.value === true);
  const blocked = async (why) => {
    assert.equal(await connection.locator('.form-status.ok').count(), 0, `${why}: a passed test shows next to the refused endpoint`);
    assert.equal(await page.locator('[data-a="sync-test"]').isDisabled(), true, `${why}: Test connection waits`);
    assert.equal(await page.locator(toggle).isDisabled(), true, `${why}: the switch waits`);
    assert.match(await connection.locator('.form-status').innerText(), /Voice didn't save the marked field\./, why);
    assert.equal(await page.locator(endpoint).getAttribute('aria-invalid'), 'true', why);
  };
  const unblocked = async () => {
    await page.waitForFunction(() => !document.querySelector('[data-a="sync-test"]').disabled);
    assert.equal(await page.locator(endpoint).getAttribute('aria-invalid'), null);
  };
  await page.click('[data-a="sync-test"]');
  await page.waitForSelector('[data-section="sync-connection"] .form-status.ok');

  // Refused when the field loses focus: the value stays to be fixed, and the passed test no longer shows.
  await blurFill(page, 'storage.sync.endpoint', 'http://example.com');
  await page.waitForSelector('.dialog');
  await page.keyboard.press('Escape');
  await page.waitForTimeout(150);
  await blocked('refused on blur');
  assert.equal(await page.inputValue(endpoint), 'http://example.com');
  await blurFill(page, 'storage.sync.endpoint', 'https://fixed.example.com');
  await unblocked();

  // Typed, then Test or the switch pressed at once while the app takes a moment to answer: the press saves the field,
  // the app refuses it, and the test or the switch that waited for that save doesn't run with the saved endpoint.
  await page.evaluate(() => (window.__settingsMockSlow.settings_set = 300));
  for (const [target, ran] of [
    ['[data-a="sync-test"]', tests],
    [toggle, switchesOn]
  ]) {
    await page.click('[data-a="sync-test"]');
    await page.waitForSelector('[data-section="sync-connection"] .form-status.ok');
    assert.equal(await page.locator(toggle).isDisabled(), false, 'tested, so the switch can turn on');
    const before = await ran();
    await page.click(endpoint);
    await page.keyboard.press('Control+A');
    await page.keyboard.type('http://typed.example.com');
    await press(page, target);
    await page.waitForSelector('.dialog');
    await page.keyboard.press('Escape');
    await page.waitForTimeout(150);
    assert.equal(await ran(), before, `${target} ran with the saved endpoint`);
    await blocked(target);
    assert.equal((await mockState()).config.storage.sync.enabled, false);
    await blurFill(page, 'storage.sync.endpoint', 'https://fixed.example.com');
    await unblocked();
  }
});

check('the sync status says what the bucket is doing', async ({ open, page }) => {
  const statusOf = async (sync) => {
    await open(`settings.html?mock=1&lang=en&page=storage&sync=${sync}`);
    return page.locator('[data-section="sync"]').innerText();
  };
  assert.match(await statusOf('on'), /Everything is in the bucket[\s\S]*Last upload/);
  assert.match(await statusOf('live'), /Audio and frames go up while the meeting records[\s\S]*3 files waiting/);
  const retrying = await statusOf('retrying');
  assert.match(retrying, /Can't reach the bucket\. Voice keeps trying\.[\s\S]*14 files waiting[\s\S]*Waiting since[\s\S]*as long as 10 GB stay free/);
  const failing = await statusOf('failing');
  assert.match(failing, /Uploads stopped\.[\s\S]*doesn't accept this access key and secret[\s\S]*41 files waiting[\s\S]*as long as 10 GB stay free/);
  assert.match(failing, /removed 3 files before they reached the bucket/);
  await page.click('[data-a="sync-details:last"]');
  assert.match(await dialogText(page), /SignatureDoesNotMatch/);
  await page.keyboard.press('Escape');
  assert.match(await statusOf('key'), /can't open its encryption key/);
  assert.equal(await page.locator('[data-section="sync"] [data-a="sync-key-use"]').count(), 1);
  assert.equal(await page.locator('[data-a="sync-key-show"]').isDisabled(), true, 'an unreadable key has nothing to show');
  await statusOf('tested');
  assert.equal(await page.locator('[data-section="sync"] .sync-status').count(), 0, 'nothing while off');
  // An upload that failed for no known reason: Error details says uploads stopped, never "The test failed.".
  await open('settings.html?mock=1&lang=en&page=storage&sync=failing');
  await page.evaluate(() => window.__settingsMockEdit((s) => (s.sync.lastError = { at: 1, code: 'failed', message: 'HTTP 500 from example.upcloudobjects.com.' })));
  await page.waitForFunction(() => !document.querySelector('[data-section="sync"]').innerText.includes('access key'));
  assert.doesNotMatch(await page.locator('[data-section="sync"]').innerText(), /The test failed/);
  await page.click('[data-a="sync-details:last"]');
  assert.match(await dialogText(page), /Uploads stopped\.[\s\S]*HTTP 500/);
  assert.doesNotMatch(await dialogText(page), /The test failed/);
});

check('Show key says what went wrong when the app can\'t give the key', async ({ open, page }) => {
  // A key that exists but can't be opened, which the page hasn't heard about yet.
  await open('settings.html?mock=1&lang=en&page=storage&sync=on');
  await page.evaluate(() => window.__settingsMockEdit((s) => (s.sync.state = 'key'), { emit: false }));
  await page.click('[data-a="sync-key-show"]');
  await page.waitForSelector('.dialog');
  assert.match(await dialogText(page), /can't open its encryption key on this computer/);
  assert.equal(await protectedNow(page), false);
  await page.keyboard.press('Escape');
  // No key yet: Show makes one, so a failure isn't an unreadable key and asks for no saved key.
  await open('settings.html?mock=1&lang=en&page=storage&sync=fresh');
  await page.evaluate(() => window.__settingsMockEdit((s) => (s.sync.state = 'key'), { emit: false }));
  await page.click('[data-a="sync-key-show"]');
  await page.waitForSelector('.dialog');
  assert.doesNotMatch(await dialogText(page), /saved key/);
  assert.match(await dialogText(page), /Couldn't change this setting\./);
  assert.equal(await page.locator('.recovery-key').count(), 0);
  assert.equal(await protectedNow(page), false);
});

check('the recovery key shows only in its protected dialog and leaves nothing behind', async ({ open, page, calls }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=on');
  // Any use of the web clipboard by the page is recorded.
  await page.evaluate(() => {
    window.__clipboardUsed = false;
    const real = navigator.clipboard;
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      get() {
        window.__clipboardUsed = true;
        return real;
      }
    });
  });
  assert.equal(await protectedNow(page), false);
  await page.click('[data-a="sync-key-show"]');
  await page.waitForSelector('.recovery-key');
  assert.equal(await protectedNow(page), true, 'hidden from capture while open');
  const log = await calls();
  const protectAt = log.findIndex(([command, args]) => command === 'settings_protect' && args.on === true);
  const showAt = log.findIndex(([command, args]) => command === 'settings_sync_key' && args.action === 'show');
  assert.ok(protectAt >= 0 && protectAt < showAt, 'protected before the code is asked for');
  assert.equal(await page.locator('.recovery-key').innerText(), ZERO_CODE);
  await page.click('[data-copy]');
  await page.waitForSelector('[data-copy]:has-text("Copied")');
  assert.ok((await calls()).some(([command]) => command === 'settings_sync_key_copy'));
  assert.equal(await page.evaluate(() => window.__clipboardUsed), false, 'never navigator.clipboard');
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('.recovery-key').count(), 0);
  assert.ok(!(await page.content()).includes('RKV1-'), 'no code in the DOM');
  assert.equal(await protectedNow(page), false, 'shown again after closing');
  assert.ok(!(await calls()).some(([command, args]) => command === 'settings_sync_key' && args.action === 'saved'), 'closing is not saving');
  // I saved it.
  await open('settings.html?mock=1&lang=en&page=storage&sync=retrying');
  assert.match(await page.locator('[data-section="sync-key"]').innerText(), /You haven't saved it yet\./);
  await page.click('[data-a="sync-key-show"]');
  await page.waitForSelector('.recovery-key');
  await page.click('.dialog .buttons .btn.primary');
  await page.waitForSelector('[data-section="sync-key"] .desc:text("Saved. Key 0f1e2d3c")');
  assert.ok((await calls()).some(([command, args]) => command === 'settings_sync_key' && args.action === 'saved'));
  assert.equal(await page.locator('.recovery-key').count(), 0);
  assert.equal(await protectedNow(page), false);
});

check('Settings never uses the web clipboard', async () => {
  const source = require('fs').readFileSync(path.join(__dirname, '..', 'settings.js'), 'utf8');
  assert.doesNotMatch(source, /navigator\s*\.\s*clipboard|execCommand\(\s*['"]copy/);
});

check('a meeting that starts recording the screen closes either key dialog', async ({ open, page }) => {
  for (const [button, selector, title] of [
    ['sync-key-show', '.recovery-key', 'Your encryption key'],
    ['sync-key-use', '.recovery-input', 'Use a key from another computer']
  ]) {
    await open('settings.html?mock=1&lang=en&page=storage&sync=on');
    await page.click(`[data-a="${button}"]`);
    await page.waitForSelector(selector);
    if (button === 'sync-key-use') await page.fill('.recovery-input', ZERO_CODE);
    assert.equal(await protectedNow(page), true);
    await page.evaluate(() => window.__settingsMockEdit((s) => (s.sync.screenRecording = true)));
    await page.waitForSelector(selector, { state: 'detached' });
    assert.ok(!(await page.content()).includes('RKV1-'), `${button}: no code left in the DOM`);
    assert.equal(await page.locator('.recovery-input').count(), 0);
    assert.match(await dialogText(page), new RegExp(`${title}[\\s\\S]*Stop the meeting's screen recording`));
    assert.equal(await protectedNow(page), false, `${button}: shown again`);
    await page.keyboard.press('Escape');
    // While it records, the dialog doesn't open at all.
    await page.click(`[data-a="${button}"]`);
    await page.waitForSelector('.dialog');
    assert.equal(await page.locator(selector).count(), 0);
    assert.match(await dialogText(page), /Stop the meeting's screen recording/);
  }
  // The app refuses Show key before the page has heard about the recording.
  await open('settings.html?mock=1&lang=en&page=storage&sync=on');
  await page.evaluate(() => window.__settingsMockEdit((s) => (s.sync.screenRecording = true), { emit: false }));
  await page.click('[data-a="sync-key-show"]');
  await page.waitForSelector('.dialog');
  assert.equal(await page.locator('.recovery-key').count(), 0);
  assert.match(await dialogText(page), /Stop the meeting's screen recording/);
  assert.equal(await protectedNow(page), false);
});

check('Use a key maps invalid_value, key_in_use and recording_screen, and takes a good key', async ({ open, page, calls }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=on');
  await page.click('[data-a="sync-key-use"]');
  await page.waitForSelector('.recovery-input');
  assert.equal(await page.getAttribute('.recovery-input', 'spellcheck'), 'false');
  assert.equal(await page.getAttribute('.recovery-input', 'autocomplete'), 'off');
  const use = '.dialog .buttons .btn.primary';
  await page.fill('.recovery-input', 'hello');
  await page.click(use);
  await page.waitForSelector('.dialog-error:text("That isn\'t a Rukoo Voice key")');
  await page.fill('.recovery-input', ZERO_CODE);
  await page.click(use);
  await page.waitForSelector('.dialog-error:text("already uploaded meetings with its own key")');
  assert.equal(await page.locator('.recovery-input').count(), 1, 'the dialog stays open on a refusal');
  await page.evaluate(() => window.__settingsMockEdit((s) => (s.sync.screenRecording = true), { emit: false }));
  await page.click(use);
  await page.waitForSelector('.recovery-input', { state: 'detached' });
  assert.match(await dialogText(page), /Stop the meeting's screen recording/);
  assert.equal(await protectedNow(page), false);
  // A computer that hasn't uploaded takes the key.
  await open('settings.html?mock=1&lang=en&page=storage&sync=fresh');
  await page.click('[data-a="sync-key-use"]');
  await page.fill('.recovery-input', ZERO_CODE.toLowerCase());
  await page.click(use);
  await page.waitForSelector('.recovery-input', { state: 'detached' });
  await page.waitForSelector('[data-section="sync-key"] .desc:text("Saved. Key 0f1e2d3c")');
  assert.ok((await calls()).some(([command, args]) => command === 'settings_sync_key' && args.action === 'import'));
  assert.equal(await protectedNow(page), false);
  assert.equal(await page.locator('.dialog').count(), 0);
});

check('Restore brings the bucket\'s meetings back with progress', async ({ open, page }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=restore');
  assert.match(await page.locator('[data-section="sync-bucket"]').innerText(), /2 meetings in your bucket aren't on this computer/);
  await page.click('[data-a="sync-restore"]');
  await page.waitForSelector('[data-section="sync-bucket"] .progress');
  assert.match(await page.locator('[data-section="sync-bucket"]').innerText(), /Restoring \d of 2/);
  await page.waitForSelector('[data-section="sync-bucket"] .title:text("Restored")', { timeout: 4000 });
  await open('settings.html?mock=1&lang=en&page=storage&sync=restore-failed');
  assert.match(await page.locator('[data-section="sync-bucket"]').innerText(), /Restoring stopped\./);
  await page.click('[data-a="sync-details:restore"]');
  assert.match(await dialogText(page), /Weekly/);
});

check('the bucket retention saves from its segmented control', async ({ open, page, mockState }) => {
  await open('settings.html?mock=1&lang=en&page=storage&sync=on');
  await page.click('[data-a="seg:storage.sync.retention:days90"]');
  assert.equal((await mockState()).config.storage.sync.retention, 'days90');
  assert.equal(await page.locator('[data-a="seg:storage.sync.retention:days90"]').getAttribute('aria-selected'), 'true');
});

check('Dutch is complete on the Storage page with sync, and in the key dialogs', async ({ open, page }) => {
  const english = [
    'Sync to your bucket',
    'In your bucket',
    'Test connection',
    'Secret access key',
    'Key saved',
    'Replace',
    'Remove',
    'Encrypted',
    'Show key',
    'Use a key',
    'Keep in the bucket',
    'Forever',
    'Everything is in the bucket',
    'files waiting',
    'Uploads stopped',
    'keeps trying',
    'Restore',
    'Waiting since',
    'Last upload',
    'saved it'
  ];
  const noEnglish = (text, where) => {
    for (const phrase of english) assert.ok(!text.includes(phrase), `"${phrase}" on the Dutch ${where}`);
  };
  for (const sync of ['fresh', 'tested', 'on', 'live', 'retrying', 'failing', 'key', 'restore', 'restore-failed']) {
    await open(`settings.html?mock=1&lang=nl&page=storage&sync=${sync}`);
    noEnglish(await page.locator('.page').innerText(), `storage page, sync=${sync}`);
  }
  await open('settings.html?mock=1&lang=nl&page=storage&sync=on');
  await page.click('[data-a="sync-key-show"]');
  await page.waitForSelector('.recovery-key');
  noEnglish(await dialogText(page), 'key dialog');
  await page.keyboard.press('Escape');
  await page.click('[data-a="sync-key-use"]');
  await page.waitForSelector('.recovery-input');
  noEnglish(await dialogText(page), 'Use a key dialog');
});

// ---------- the pill ----------

check('the pill reports the stop button for the native window, and null without one', async ({ open, page }) => {
  await open('pill.html?mock=meeting-recording&lang=en');
  await page.waitForTimeout(400);
  let layout = await page.evaluate(() => window.__pillLayout);
  // 28px, rounded outwards: the centred pill can start at a fractional x.
  assert.ok([28, 29].includes(layout.button.width), `width ${layout.button.width}`);
  assert.equal(layout.button.height, 28);
  assert.equal(layout.button.y, 28, '4px inside a 36px pill that starts 24px down');
  assert.ok(layout.button.x + layout.button.width <= layout.pill.x + layout.pill.width - 3, 'inside the pill, 4px from its end');
  await page.click('.stop');
  await page.waitForTimeout(400);
  layout = await page.evaluate(() => window.__pillLayout);
  assert.ok(layout.button.width > 40, 'the widened Stop? button is reported');
  await open('pill.html?mock=listening&lang=en');
  await page.waitForTimeout(400);
  layout = await page.evaluate(() => window.__pillLayout);
  assert.equal(layout.button, null);
});

check('the screen button sits before stop and opens its native menu even with capture Off', async ({ open, page }) => {
  await open('pill.html?mock=meeting-recording&lang=en');
  await page.waitForTimeout(400);
  assert.equal((await page.evaluate(() => window.__pillLayout)).screenButton, null, 'no screenSwitch, no monitor button');
  await open('pill.html?mock=meeting-screen-button&lang=en');
  await page.waitForTimeout(400);
  const layout = await page.evaluate(() => window.__pillLayout);
  assert.ok([28, 29].includes(layout.screenButton.width), `width ${layout.screenButton.width}`);
  assert.equal(layout.screenButton.height, 28);
  assert.equal(layout.screenButton.y, layout.button.y, 'on one line with stop');
  assert.ok(layout.button.x - (layout.screenButton.x + layout.screenButton.width) <= 5, 'right before the stop button');
  assert.ok(layout.screenButton.x > layout.pill.x, 'inside the pill');
  assert.equal(await page.locator('.screen-btn').getAttribute('aria-label'), 'Screen recording');
  // The pointer colours the button it is over, and only that one.
  await page.evaluate(() => window.__pillMock.emit('pill://pointer', { hover: true, pressed: false, target: 'screen' }));
  assert.equal(await page.locator('.screen-btn.hover').count(), 1);
  assert.equal(await page.locator('.stop.hover').count(), 0);
  await page.evaluate(() => window.__pillMock.emit('pill://pointer', { hover: true, pressed: false }));
  assert.equal(await page.locator('.stop.hover').count(), 1, 'no target means the stop window, as before');
  await page.click('.screen-btn');
  assert.equal(await page.evaluate(() => window.__pillMock.screenMenu.opened), 1);
  await open('pill.html?mock=meeting-screen-off&lang=nl');
  await page.waitForTimeout(400);
  assert.equal(await page.locator('.screen-btn').getAttribute('aria-label'), 'Schermopname');
  assert.ok((await page.evaluate(() => window.__pillLayout)).screenButton);
});

check('the screen notices read right in both languages, with the screen glyph', async ({ open, page }) => {
  const notices = {
    'meeting-video-dropped': ['Video stopped: disk almost full', 'Video gestopt: schijf bijna vol'],
    'meeting-video-dropped-encoder': ['Video stopped: no encoder', 'Video gestopt: geen encoder'],
    'meeting-video-dropped-device': ['Video stopped: graphics card', 'Video gestopt: videokaart'],
    'meeting-frames-dropped': ['Screen stopped: disk almost full', 'Scherm gestopt: schijf bijna vol'],
    'meeting-screen-failed': ['Screen recording stopped', 'Schermopname gestopt'],
    'meeting-screen-moved': ['Screen: DELL U2720Q', 'Scherm: DELL U2720Q']
  };
  for (const [state, texts] of Object.entries(notices)) {
    for (const [i, lang] of ['en', 'nl'].entries()) {
      await open(`pill.html?mock=${state}&lang=${lang}`);
      assert.equal(await page.locator('.label:not(.out)').innerText(), texts[i], `${state} in ${lang}`);
      assert.equal(await page.locator('.ind [data-icon="screen"]').count(), 1, `${state} shows the screen glyph`);
    }
  }
  assert.equal(await page.locator('.ind.tone-text2').count(), 1, 'a move is news, not a warning');
});

check('a meeting snapshot with an unknown screen mode is ignored', async ({ open, page }) => {
  await open('pill.html?mock=meeting-recording&lang=en');
  await page.evaluate(() =>
    window.__pillMock.emit('pill://snapshot', {
      seq: 900,
      visible: true,
      phase: 'meeting',
      activity: 'dictate',
      notice: { key: 'pill.screenFailed', args: {} },
      meeting: { state: 'recording', atMs: 1000, screen: '4k', screenSwitch: true, compact: false, confirmStop: false, stopEnabled: true, rukooConnected: true },
      edge: 'bottom',
      theme: 'dark',
      locale: 'en'
    })
  );
  await page.waitForTimeout(300);
  assert.equal(await page.locator('.label:not(.out)').innerText(), 'Recording');
});

check('two clicks stop a meeting; one only asks', async ({ open, page }) => {
  await open('pill.html?mock=meeting-recording&lang=en');
  await page.click('.stop');
  await page.waitForSelector('.stop.confirm');
  assert.equal(await page.locator('.stop-label').innerText(), 'Stop?');
  await page.click('.stop');
  await page.waitForSelector('.label:text("Saving")');
});

check('a snapshot with the wrong shape is ignored, an older one too', async ({ open, page }) => {
  await open('pill.html?mock=listening&lang=en');
  await page.evaluate(() => {
    window.__pillMock.emit('pill://snapshot', { seq: 999, visible: true, phase: 'exploding' });
    window.__pillMock.emit('pill://snapshot', { seq: 0, visible: false, phase: 'hidden', activity: 'dictate', edge: 'bottom', theme: 'dark', locale: 'en' });
  });
  await page.waitForTimeout(300);
  assert.equal(await page.locator('.label:not(.out)').innerText(), 'Listening');
});

check('an unknown notice falls back instead of breaking the pill', async ({ open, page }) => {
  await open('pill.html?mock=listening&lang=en');
  await page.evaluate(() =>
    window.__pillMock.emit('pill://snapshot', { seq: 50, visible: true, phase: 'error', activity: 'dictate', notice: { key: 'pill.error.fromTheFuture', args: {} }, edge: 'bottom', theme: 'dark', locale: 'en' })
  );
  await page.waitForSelector('.label.error:text("Try again")');
});

check('the meeting clock ticks from atMs and shows hours from an hour on', async ({ open, page }) => {
  await open('pill.html?mock=meeting-long&lang=en');
  const first = await page.locator('.time').innerText();
  assert.match(first, /^1:02:3\d$/);
  await page.waitForTimeout(1200);
  assert.notEqual(await page.locator('.time').innerText(), first);
});

check('with reduced motion the level still shows, in three steps', async ({ open, page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  try {
    await open('pill.html?mock=listening-sound&lang=en');
    await page.waitForTimeout(1500);
    const scales = await page.$$eval('.bar', (bars) => bars.map((bar) => bar.style.transform));
    assert.ok(scales.every((s) => ['scaleY(0.35)', 'scaleY(0.6)', 'scaleY(1)'].includes(s)), scales.join());
    assert.equal(await page.$eval('.bar', (bar) => getComputedStyle(bar).animationName), 'none');
    // The other loops stop too, and their bars rest at their rest heights.
    for (const state of ['arming', 'no-audio', 'transcribing']) {
      await open(`pill.html?mock=${state}&lang=en`);
      const names = await page.$$eval('.bar', (bars) => bars.map((bar) => getComputedStyle(bar).animationName));
      assert.deepEqual([...new Set(names)], ['none'], `${state} still animates`);
    }
    await open('pill.html?mock=transcribing&lang=en');
    assert.equal(await page.$eval('.label', (label) => getComputedStyle(label).backgroundImage), 'none', 'the shimmer stops');
  } finally {
    await page.emulateMedia({ reducedMotion: null });
  }
});

// ---------- runner ----------

async function main() {
  const server = await start(0);
  const base = `http://127.0.0.1:${server.address().port}/`;
  const env = { ...process.env };
  delete env.ELECTRON_RUN_AS_NODE;
  const app = await electron.launch({ args: [path.join(__dirname, 'electron-main.js')], env });
  let failed = 0;
  try {
    const page = await app.firstWindow();
    const errors = [];
    page.on('pageerror', (e) => errors.push(e.message));
    await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setContentSize(760, 680));
    const open = async (url) => {
      const pill = url.startsWith('pill');
      await app.evaluate(({ BrowserWindow }, [w, h]) => BrowserWindow.getAllWindows()[0].setContentSize(w, h), pill ? [384, 100] : [760, 680]);
      await page.goto(base + url);
      // Records what the page sent to settings_save_provider, as the app would receive it.
      if (!pill) {
        await page.waitForSelector('.page-inner .card');
        await page.evaluate(() => {
          window.__lastSave = null;
        });
      } else await page.waitForSelector('.pill');
    };
    const mockState = () => page.evaluate(() => window.__settingsMockState());
    // Every command the page sent the mock since it loaded: [command, args].
    const calls = () => page.evaluate(() => window.__settingsMockCalls);
    for (const { name, run } of checks) {
      errors.length = 0;
      try {
        await run({ open, page, mockState, calls });
        assert.deepEqual(errors, [], 'page errors');
        console.log(`ok   ${name}`);
      } catch (error) {
        failed += 1;
        console.log(`FAIL ${name}\n     ${String(error.message).split('\n').join('\n     ')}`);
      }
    }
  } finally {
    await app.close();
    server.close();
  }
  console.log(`${checks.length - failed} of ${checks.length} checks passed`);
  process.exitCode = failed ? 1 : 0;
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
