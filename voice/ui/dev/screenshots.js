// Screenshots of every pill state and every Settings page, in light and dark, English and Dutch, from the mocks.
// Development only; the images are verification evidence for the GitHub issue and never go into Git.
//   node voice/ui/dev/screenshots.js <outDir> [--scale 1.5] [--only pill|settings]
// Runs the repository's Electron through Playwright with a bare app (electron-main.js) whose window sits off-screen
// and never takes focus. Settings pages are captured at their full height, at 760 or 480 CSS pixels wide.
'use strict';

const { _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const { start } = require('./serve.js');

const args = process.argv.slice(2);
const option = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : null);
const out = path.resolve(args.find((arg, i) => !arg.startsWith('--') && !['--scale', '--only'].includes(args[i - 1])) || 'tmp/voice-ui-shots');
const scale = Number(option('--scale')) || 1.5;
const only = option('--only');

const THEMES = ['light', 'dark'];
const LANGS = ['en', 'nl'];
const PILL_STATES = [
  'arming',
  'listening',
  'listening-sound',
  'listening-edit',
  'no-audio',
  'warning',
  'transcribing',
  'loading-model',
  'rewriting',
  'inserted',
  'inserted-one',
  'rewrote',
  'nothing-heard',
  'error-offline',
  'error-no-key',
  'error-auth',
  'error-timeout',
  'error-mic',
  'error-model',
  'error-model-missing',
  'error-blocked',
  'error-select-text',
  'error-selection-too-long',
  'error-generic',
  'meeting-recording',
  'meeting-compact',
  'meeting-long',
  'meeting-paused',
  'meeting-confirm',
  'meeting-hover',
  'meeting-saving',
  'meeting-failed',
  'meeting-mic-lost',
  'meeting-mic-back',
  'meeting-video-dropped',
  'meeting-video-dropped-encoder',
  'meeting-frames-dropped',
  'meeting-screen-failed',
  'meeting-screen-moved',
  'meeting-screen-button',
  'meeting-screen-off',
  'meeting-screen-change-failed',
  'meeting-screen-hover',
  'meeting-dictating'
];
const SETTINGS_PAGES = ['main', 'transcription', 'dictionary', 'fillers', 'rewriting', 'meetingTranscription', 'speakers', 'storage', 'rukoo', 'import'];
// Other states worth seeing: [file name, query].
const SCENARIOS = [
  ['main-fresh', 'page=main&scenario=fresh'],
  ['main-busy', 'page=main&scenario=busy'],
  ['main-failed', 'page=main&scenario=failed'],
  ['main-dev', 'page=main&scenario=dev'],
  ['main-cloud', 'page=meetings&scenario=cloud'],
  ['transcription-missing', 'page=transcription&scenario=fresh'],
  ['transcription-downloading', 'page=transcription&scenario=busy'],
  ['transcription-failed', 'page=transcription&scenario=failed'],
  ['transcription-cloud', 'page=transcription&scenario=cloud'],
  ['meeting-transcription-consent', 'page=meetingTranscription&scenario=cloud'],
  ['dictionary-empty', 'page=dictionary&scenario=fresh'],
  ['rukoo-not-running', 'page=rukoo&scenario=fresh'],
  ['rukoo-outdated', 'page=rukoo&scenario=busy'],
  ['import-never', 'page=import&scenario=fresh'],
  ['import-dev', 'page=import&scenario=dev'],
  ['import-cloud', 'page=import&scenario=cloud'],
  ['main-screen', 'page=meetings&features=screen'],
  ['main-screen-limits', 'page=meetings&noEncoder=1&noOcr=1'],
  ['main-screen-unavailable', 'page=meetings&screenUnavailable=1'],
  ['storage-screen', 'page=storage&features=screen'],
  ['main-speakers-cpu', 'page=meetings&speakers=cpu'],
  ['permissions', 'page=permissions&platform=macos'],
  ['permissions-denied', 'page=permissions&platform=macos&permissions=denied'],
  ['speakers-ready', 'page=speakers&speakers=ready'],
  ['speakers-off', 'page=speakers&speakers=off'],
  ['speakers-cpu', 'page=speakers&speakers=cpu'],
  ['speakers-model', 'page=speakers&speakers=model'],
  ['speakers-modelDamaged', 'page=speakers&speakers=modelDamaged'],
  ['speakers-runtime', 'page=speakers&speakers=runtime'],
  ['speakers-runtimeFailed', 'page=speakers&speakers=runtimeFailed'],
  ['speakers-platform', 'page=speakers&speakers=platform'],
  ['speakers-key', 'page=speakers&speakers=key'],
  ['speakers-consent', 'page=speakers&speakers=consent'],
  ['speakers-mistral', 'page=speakers&speakers=mistral'],
  ['speakers-stopped', 'page=speakers&speakers=stopped'],
  ['meetingTranscription-stream', 'page=meetingTranscription&speakers=consent'],
  ['later-main', 'page=meetings&later=1'],
  ['later-speakers', 'page=speakers&later=1'],
  // Settings > Storage with bucket sync in each of its states (mock/settings.js, &sync=).
  ...['fresh', 'tested', 'on', 'live', 'retrying', 'failing', 'paused', 'unavailable', 'key', 'restore', 'restore-failed'].map((sync) => [`storage-sync-${sync}`, `page=storage&sync=${sync}`])
];

async function main() {
  const server = await start(0);
  const base = `http://127.0.0.1:${server.address().port}/`;
  const env = { ...process.env };
  delete env.ELECTRON_RUN_AS_NODE;
  const app = await electron.launch({ args: [path.join(__dirname, 'electron-main.js'), `--force-device-scale-factor=${scale}`], env });
  const problems = [];
  const saved = [];
  try {
    const page = await app.firstWindow();
    page.on('pageerror', (e) => problems.push(`${page.url()}: ${e.message}`));
    page.on('console', (m) => m.type() === 'error' && problems.push(`${page.url()}: ${m.text()}`));
    const size = (w, h) => app.evaluate(({ BrowserWindow }, [w, h]) => BrowserWindow.getAllWindows()[0].setContentSize(w, h), [w, h]);
    const shot = async (file) => {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      await page.screenshot({ path: file });
      saved.push(file);
    };

    if (only !== 'settings') {
      for (const theme of THEMES)
        for (const lang of LANGS) {
          await size(384, 100);
          for (const state of PILL_STATES) {
            await page.goto(`${base}pill.html?mock=${state}&theme=${theme}&lang=${lang}&backdrop=1`);
            await page.waitForSelector('.pill');
            await page.waitForTimeout(650);
            await shot(path.join(out, 'pill', `${theme}-${lang}`, `${state}.png`));
          }
          // Where the app puts the native stop and monitor windows, from the page's own pill_layout report.
          for (const state of ['meeting-recording', 'meeting-confirm', 'meeting-screen-button']) {
            await page.goto(`${base}pill.html?mock=${state}&theme=${theme}&lang=${lang}&backdrop=1&hit=1`);
            await page.waitForTimeout(700);
            await shot(path.join(out, 'pill', `${theme}-${lang}`, `${state}-hit.png`));
          }
          await size(1240, 1600);
          await page.goto(`${base}dev/gallery.html?theme=${theme}&lang=${lang}`);
          await page.waitForTimeout(1800);
          await fitHeight(page, size, 1240, 'body');
          await shot(path.join(out, 'pill', `gallery-${theme}-${lang}.png`));
        }
      await size(1240, 1600);
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.goto(`${base}dev/gallery.html?theme=dark&lang=en`);
      await page.waitForTimeout(1800);
      await fitHeight(page, size, 1240, 'body');
      await shot(path.join(out, 'pill', 'gallery-dark-en-reduced-motion.png'));
      await page.emulateMedia({ reducedMotion: null });
    }

    if (only !== 'pill') {
      const settings = async (file, query, width = 760) => {
        await size(width, 680);
        await page.goto(`${base}settings.html?mock=1&${query}`);
        await page.waitForSelector('.page-inner .card');
        await page.waitForTimeout(500);
        await fitHeight(page, size, width, '.page-inner', 52 + 4 + 48);
        await shot(file);
      };
      for (const theme of THEMES)
        for (const lang of LANGS) {
          for (const name of SETTINGS_PAGES) await settings(path.join(out, 'settings', `${theme}-${lang}`, `${name}.png`), `page=${name}&theme=${theme}&lang=${lang}`);
          for (const [name, query] of SCENARIOS) await settings(path.join(out, 'settings', `${theme}-${lang}`, `${name}.png`), `${query}&theme=${theme}&lang=${lang}`);
          // The macOS key picker: Mac names, and only keys a Mac keyboard has.
          await size(760, 680);
          await page.goto(`${base}settings.html?mock=1&page=main&platform=macos&theme=${theme}&lang=${lang}`);
          await page.waitForSelector('[data-a="choose:dictation.hotkey"]');
          await page.click('[data-a="choose:dictation.hotkey"]');
          await page.waitForTimeout(300);
          await shot(path.join(out, 'settings', `${theme}-${lang}`, 'dictation-key-macos.png'));
          await page.keyboard.press('Escape');
        }
      // A narrow window, at Settings' minimum width.
      for (const [theme, lang] of [
        ['dark', 'nl'],
        ['light', 'en'],
        ['light', 'nl']
      ]) {
        for (const [name, query] of [
          ['main', 'page=main&scenario=busy'],
          ['main-screen-limits', 'page=meetings&noEncoder=1&noOcr=1'],
          ['permissions', 'page=permissions&platform=macos'],
          ['permissions-denied', 'page=permissions&platform=macos&permissions=denied'],
          ['transcription', 'page=transcription&scenario=busy'],
          ['meeting-transcription', 'page=meetingTranscription&scenario=cloud'],
          ['meeting-transcription-stream', 'page=meetingTranscription&speakers=consent'],
          ['speakers-cpu', 'page=speakers&speakers=cpu'],
          ['speakers-mistral', 'page=speakers&speakers=mistral'],
          ['dictionary', 'page=dictionary'],
          ['rukoo', 'page=rukoo'],
          ['import', 'page=import&scenario=cloud'],
          ['storage-sync-failing', 'page=storage&sync=failing'],
          ['storage-sync-key', 'page=storage&sync=key'],
          ['storage-sync-restore', 'page=storage&sync=restore']
        ])
          await settings(path.join(out, 'settings', `narrow-${theme}-${lang}`, `${name}.png`), `${query}&theme=${theme}&lang=${lang}`, 480);
      }
      // Dialogs, opened the way a user opens them.
      for (const [theme, lang] of [
        ['dark', 'en'],
        ['light', 'nl']
      ]) {
        const dir = path.join(out, 'settings', `dialogs-${theme}-${lang}`);
        await size(760, 680);
        await page.goto(`${base}settings.html?mock=1&page=main&scenario=busy&theme=${theme}&lang=${lang}`);
        await page.waitForSelector('.page-inner .card');
        await page.click('[data-a="choose:dictation.hotkey"]');
        await page.waitForTimeout(300);
        await shot(path.join(dir, 'dictation-key.png'));
        await page.keyboard.press('Escape');
        await page.click('[data-a="licenses"]');
        await page.waitForTimeout(300);
        await shot(path.join(dir, 'licenses.png'));
        await page.keyboard.press('Escape');
        // Why the speaker label runtime didn't start.
        await page.goto(`${base}settings.html?mock=1&page=speakers&speakers=runtimeFailed&theme=${theme}&lang=${lang}`);
        await page.waitForSelector('[data-a="speakers-details"]');
        await page.click('[data-a="speakers-details"]');
        await page.waitForTimeout(300);
        await shot(path.join(dir, 'speakers-error-details.png'));
        await page.keyboard.press('Escape');
        await page.goto(`${base}settings.html?mock=1&page=import&theme=${theme}&lang=${lang}`);
        await page.waitForSelector('.page-inner .card');
        await page.click('[data-a="import"]');
        await page.waitForTimeout(300);
        await shot(path.join(dir, 'import-again.png'));
        await page.keyboard.press('Escape');
        // A provider form after Refresh, and after a failed save.
        await page.goto(`${base}settings.html?mock=1&page=rewriting&theme=${theme}&lang=${lang}`);
        await page.waitForSelector('[data-a="refresh:rewriting"]');
        await page.click('[data-a="refresh:rewriting"]');
        await page.waitForTimeout(1000);
        await page.fill('[data-field="rewriting.model"]', 'fail');
        await page.click('[data-a="save:rewriting"]');
        await page.waitForTimeout(900);
        await fitHeight(page, size, 760, '.page-inner', 52 + 4 + 48);
        await shot(path.join(dir, 'rewriting-save-failed.png'));
        await size(760, 680);
        await page.click('[data-a="details:rewriting"]');
        await page.waitForTimeout(300);
        await shot(path.join(dir, 'rewriting-error-details.png'));
        await page.keyboard.press('Escape');
        // A dictionary entry that's refused.
        await page.goto(`${base}settings.html?mock=1&page=dictionary&theme=${theme}&lang=${lang}`);
        await page.waitForSelector('[data-a="dict-mode"]');
        await page.click('[data-a="dict-mode"]');
        await page.fill('[data-field="dict.from"]', 'rukoo');
        await page.click('[data-a="dict-add"]');
        await page.waitForTimeout(200);
        await shot(path.join(dir, 'dictionary-error.png'));
      }
      // Bucket sync's dialogs and Test connection, in both themes and languages: the recovery code (zeros in the
      // mock), copied; a refused key; a failed test with its Error details; a test that found another key.
      for (const theme of THEMES)
        for (const lang of LANGS) {
          const dir = path.join(out, 'settings', `sync-${theme}-${lang}`);
          const storage = async (sync) => {
            await size(760, 680);
            await page.goto(`${base}settings.html?mock=1&page=storage&sync=${sync}&theme=${theme}&lang=${lang}`);
            await page.waitForSelector('[data-section="sync"]');
          };
          await storage('on');
          await page.click('[data-a="sync-key-show"]');
          await page.waitForSelector('.recovery-key');
          await page.waitForTimeout(300);
          await shot(path.join(dir, 'key-show.png'));
          await page.click('[data-copy]');
          await page.waitForTimeout(200);
          await shot(path.join(dir, 'key-copied.png'));
          await page.keyboard.press('Escape');
          await page.click('[data-a="sync-key-use"]');
          await page.waitForSelector('.recovery-input');
          await page.waitForTimeout(300);
          await shot(path.join(dir, 'key-use.png'));
          await page.fill('.recovery-input', 'RKV1-00000-00000-00000-00000-00000-00000-00000-00000-00000-00000-00000');
          await page.click('.dialog .buttons .btn.primary');
          await page.waitForSelector('.dialog-error:not([hidden])');
          await shot(path.join(dir, 'key-use-in-use.png'));
          await page.keyboard.press('Escape');
          // A meeting starts recording the screen while the key shows.
          await page.click('[data-a="sync-key-show"]');
          await page.waitForSelector('.recovery-key');
          await page.evaluate(() => window.__settingsMockEdit((s) => (s.sync.screenRecording = true)));
          await page.waitForSelector('.recovery-key', { state: 'detached' });
          await page.waitForTimeout(300);
          await shot(path.join(dir, 'key-closed-by-screen-recording.png'));
          await storage('tested');
          await page.fill('[data-field="storage.sync.bucket"]', 'fail');
          await page.locator('[data-field="storage.sync.bucket"]').blur();
          await page.click('[data-a="sync-test"]');
          await page.waitForSelector('.form-status.bad');
          await fitHeight(page, size, 760, '.page-inner', 52 + 4 + 48);
          await shot(path.join(dir, 'test-failed.png'));
          await size(760, 680);
          await page.click('[data-a="sync-details:test"]');
          await page.waitForTimeout(300);
          await shot(path.join(dir, 'test-failed-details.png'));
          await page.keyboard.press('Escape');
          await page.fill('[data-field="storage.sync.bucket"]', 'shared');
          await page.locator('[data-field="storage.sync.bucket"]').blur();
          await page.click('[data-a="sync-test"]');
          await page.waitForSelector('.form-status.ok');
          await fitHeight(page, size, 760, '.page-inner', 52 + 4 + 48);
          await shot(path.join(dir, 'test-other-keys.png'));
        }
    }
  } finally {
    await app.close();
    server.close();
  }
  console.log(`${saved.length} screenshots in ${out}`);
  if (problems.length) {
    console.log(`Page errors:\n${[...new Set(problems)].join('\n')}`);
    process.exitCode = 1;
  }
}

// Grows the window to the content's full height, so one image shows the whole page.
async function fitHeight(page, size, width, selector, extra = 40) {
  const height = await page.evaluate((sel) => Math.ceil(document.querySelector(sel).scrollHeight), selector);
  await size(width, Math.min(4000, height + extra));
  await page.waitForTimeout(250);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
