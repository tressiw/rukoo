// The calendar tools in the chat panel: the real app with the demo account and the fake agents (SEM_AGENT_FAKE=1).
// "schedule" makes the fake agent look for free time and ask Rukoo to add a call there with create_event. Screenshots
// for review go to tmp/e2e-shots/agent-calendar in the checkout, which Git ignores.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

const SHOTS = path.join(__dirname, '..', '..', 'tmp', 'e2e-shots', 'agent-calendar');
let app;
let win;
let dataDir;

test.beforeEach(async () => {
  dataDir = tempDir('rukoo-agent-calendar-e2e-');
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1', SEM_AGENT_FAKE: '1', SEM_AGENT_FAKE_SCALE: '0.5' };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await electron.launch({ args: ['.'], cwd: path.join(__dirname, '..', '..'), env });
  win = await app.firstWindow();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 950));
  await win.waitForSelector('.provider-grid');
  await win.click('[data-s="demo"]');
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
});

test.afterEach(async () => {
  await closeApp(app);
  removeDir(dataDir);
});

const pane = () => win.locator('.agentpane');
const transcript = () => win.locator('.agentpane .ap-transcript');
const input = () => win.locator('.agentpane .bui-pb__ta');

async function openChat(subject) {
  await win.locator('.item', { hasText: subject }).first().click();
  await expect(win.locator('.reader-subject')).toHaveText(subject);
  await win.keyboard.press('Control+j');
  await expect(pane()).toBeVisible();
  await expect(input()).toBeFocused();
}

async function say(text) {
  await input().fill(text);
  await input().press('Enter');
}

async function setTheme(theme) {
  await win.evaluate((t) => window.mail.call('updateSettings', { theme: t }), theme);
  await expect(win.locator('html')).toHaveClass(theme === 'light' ? /light/ : /^(?!.*light).*$/);
}

async function shot(name) {
  fs.mkdirSync(SHOTS, { recursive: true });
  for (const theme of ['dark', 'light']) {
    await setTheme(theme);
    await win.waitForTimeout(600);
    await pane().screenshot({ path: path.join(SHOTS, `${name}-${theme}.png`) });
  }
  await setTheme('dark');
}

// The events titled title in the next 16 days, as the calendar store has them.
const events = (title) =>
  app.evaluate(async (_e, title) => {
    const now = Date.now();
    const { events: list } = await global.__semCalendars.read({ start: now, end: now + 16 * 86400000, maxAgeMs: 0 });
    return list.filter((e) => e.title === title).map((e) => ({ start: e.start, end: e.end, location: e.location, calendarId: e.calendarId }));
  }, title);

test('an event the agent asks for waits on a card that says what it adds, and Create adds exactly that', async () => {
  await openChat('Call on Thursday');
  await say('Schedule a call with Sanne');
  const card = transcript().locator('.bui-approval');
  await expect(card.locator('.bui-approval__title')).toHaveText('Hermes wants to add an event to your calendar');
  await expect(card.locator('.bui-approval__k')).toHaveText(['Title', 'When', 'Calendar', 'Location']);
  const values = card.locator('.bui-approval__v');
  await expect(values.nth(0)).toHaveText('Call with Sanne');
  await expect(values.nth(1)).toHaveText(/^\w{3}, \d{1,2} \w{3} \d{4}, \d\d:\d\d\s*–\s*\d\d:\d\d$/);
  await expect(values.nth(2)).toHaveText('Demo User (demo@example.com)');
  await expect(values.nth(3)).toHaveText('Phone');
  await expect(card).toContainText('Waiting for you');
  await expect(card.getByRole('button', { name: 'Cancel', exact: true })).toBeVisible();
  // The free-time chip stays; the event has its own card, so no chip repeats it.
  await expect(transcript()).toContainText('Rukoo asks you before it adds the call.');
  await expect(transcript().locator('.bui-tool')).toHaveText(['Looked for free time']);
  expect(await events('Call with Sanne')).toEqual([]);
  await shot('01-card');

  await card.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(card).toContainText('Approved');
  // Shrunk to its title, the card still names the event.
  await expect(card.locator('.bui-approval__title')).toHaveText('Add "Call with Sanne" to your calendar');
  await expect(transcript()).toContainText('Added "Call with Sanne" to your calendar');
  const made = await events('Call with Sanne');
  expect(made).toHaveLength(1);
  expect([made[0].end - made[0].start, made[0].location, made[0].calendarId]).toEqual([30 * 60000, 'Phone', 'personal']);
  await shot('02-added');
});

test('the calendar card speaks Dutch, and Cancel adds nothing', async () => {
  await win.evaluate(() => window.mail.call('updateSettings', { language: 'nl' }));
  await expect(win.locator('.list-title h1')).toHaveText('Postvak IN');
  await openChat('Call on Thursday');
  await say('Schedule a call with Sanne');
  const card = transcript().locator('.bui-approval');
  await expect(card.locator('.bui-approval__title')).toHaveText('Hermes wil een afspraak in je agenda zetten');
  await expect(card.locator('.bui-approval__k')).toHaveText(['Titel', 'Wanneer', 'Agenda', 'Locatie']);
  await expect(card).toContainText('Wacht op jou');
  await expect(card.getByRole('button', { name: 'Aanmaken', exact: true })).toBeVisible();
  await expect(transcript().locator('.bui-tool')).toHaveText(['Zocht naar vrije tijd']);
  await shot('03-card-nl');
  await card.getByRole('button', { name: 'Annuleren', exact: true }).click();
  await expect(card).toContainText('Afgewezen');
  await expect(card.locator('.bui-approval__title')).toHaveText('"Call with Sanne" in je agenda zetten');
  // The agent hears it with the next message; the calendar has nothing new.
  await expect
    .poll(() => app.evaluate(() => global.__semAgents.list()[0].id).then((cid) => app.evaluate((_e, id) => global.__semAgents.conversations.get(id).notes.join('\n'), cid)))
    .toMatch(/The user declined: <unsafe_content source="calendar">Add "Call with Sanne" to the calendar<\/unsafe_content>/);
  expect(await events('Call with Sanne')).toEqual([]);
});

// #312: the email stays open in Mail behind the calendar, but a chat started in the calendar isn't about it.
test('a chat started in the calendar leaves out the email left open in Mail', async ({}, testInfo) => {
  const chip = () => pane().locator('.bui-pb .bui-entity');
  const heading = () => pane().locator('.bui-rec__title');
  await setTheme('dark');
  await openChat('Your parcel is on its way');
  await expect(chip().locator('.bui-entity__name')).toHaveText('Your parcel is on its way');
  await expect(heading()).toHaveText('What should Hermes do with this email?');

  await win.click('[data-mode="calendar"]');
  await expect(win.locator('.calendar')).toBeVisible();
  await expect(heading()).toHaveText('Ask Hermes about your calendar');
  await expect(chip()).toHaveCount(0);
  await say('Schedule a call with Sanne');
  await expect(transcript().locator('.bui-approval')).toBeVisible();
  await expect(transcript()).toContainText('Rukoo asks you before it adds the call.');
  // Named after its first message, as a chat without an email is.
  await expect(pane().locator('.ap-title')).toHaveText('Schedule a call with Sanne');
  await expect(chip()).toHaveCount(0);
  expect(await app.evaluate(() => global.__semAgents.list().map((c) => ({ title: c.title, message: c.message })))).toEqual([{ title: 'Schedule a call with Sanne', message: null }]);
  await win.screenshot({ path: testInfo.outputPath('calendar-chat-en-dark.png'), animations: 'disabled' });

  // New chat starts without the email too.
  await win.click('.agentpane [data-ap="new"]');
  await expect(heading()).toHaveText('Ask Hermes about your calendar');
  await expect(chip()).toHaveCount(0);
  await win.evaluate(() => window.mail.call('updateSettings', { language: 'nl', theme: 'light' }));
  await expect(heading()).toHaveText('Vraag Hermes iets over je agenda');
  await expect(win.locator('html')).toHaveClass(/light/);
  await win.screenshot({ path: testInfo.outputPath('calendar-new-chat-nl-light.png'), animations: 'disabled' });

  // Back in Mail the panel follows the open email again.
  await win.evaluate(() => window.mail.call('updateSettings', { language: 'en' }));
  await win.click('[data-mode="mail"]');
  await expect(win.locator('.reader-subject')).toHaveText('Your parcel is on its way');
  await expect(chip().locator('.bui-entity__name')).toHaveText('Your parcel is on its way');
  await expect(heading()).toHaveText('What should Hermes do with this email?');
});

for (const method of ['findFor', 'forPanel']) for (const close of [false, true]) {
  const scenario = close ? 'reopening after a closed Calendar visit retries the Mail chat lookup' : 'switching to Calendar leaves a pending email chat lookup behind';
  test(`${scenario}: ${method}`, async () => {
    const subject = 'Your parcel is on its way';
    const messageId = await win.locator('.item', { hasText: subject }).first().getAttribute('data-id');
    await app.evaluate((_e, { method, messageId, subject }) => {
      const hub = global.__semAgents;
      const conversation = hub.create({ agent: 'hermes', message: { id: messageId, subject } });
      const original = hub[method].bind(hub);
      const releases = [];
      global.__semCalendarLookup = {
        releases,
        release() {
          hub[method] = original;
          for (const resolve of releases) resolve();
        }
      };
      hub[method] = (...args) => {
        const result = original(...args);
        const target = method === 'findFor' ? args[0].id === messageId : args[0] === conversation.id;
        if (!target) return result;
        return new Promise((resolve) => releases.push(() => resolve(result)));
      };
    }, { method, messageId, subject });
    await openChat(subject);
    await expect.poll(() => app.evaluate(() => global.__semCalendarLookup.releases.length)).toBeGreaterThan(0);

    if (close) {
      await pane().locator('[data-ap="close"]').click();
      await expect(pane()).toBeHidden();
    }
    await win.click('[data-mode="calendar"]');
    await expect(win.locator('.calendar')).toBeVisible();
    await app.evaluate(() => global.__semCalendarLookup.release());
    await win.evaluate(async () => {
      // Allow the delivered lookup and any follow-up call for its transcript to reach the renderer.
      await window.mail.call('agentList');
      await window.mail.call('agentList');
    });
    if (!close) {
      await expect(pane().locator('.bui-rec__title')).toHaveText('Ask Hermes about your calendar');
      await expect(pane().locator('.bui-pb .bui-entity')).toHaveCount(0);
    }

    await win.click('[data-mode="mail"]');
    if (close) await win.keyboard.press('Control+j');
    await expect(pane().locator('.ap-title')).toHaveText(subject);
  });
}
