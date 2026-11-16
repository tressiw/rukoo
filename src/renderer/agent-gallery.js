// Dev gallery for the agent panel components: a static column per component state and a live column
// that streams, ticks and resolves. scripts/agent-gallery.js screenshots it in both themes.
import * as bui from './agent/bui.js';

const $ = (sel) => document.querySelector(sel);
const col = (title) => {
  const el = document.createElement('div');
  el.className = 'gcol bui';
  el.dataset.col = title;
  $('#gallery').append(el);
  return el;
};
const head = (parent, text) => {
  const el = document.createElement('h2');
  el.textContent = text;
  parent.append(el);
};
const row = (...items) => {
  const el = document.createElement('div');
  el.className = 'grow';
  el.append(...items);
  return el;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const AGENTS = [
  { id: 'clark', name: 'Clark', tag: 'Hermes', status: 'ready' },
  { id: 'claude', name: 'Claude', tag: 'Claude Code', status: 'ready' },
  { id: 'codex', name: 'Codex', tag: 'Codex', status: 'offline' },
  { id: 'openclaw', name: 'OpenClaw', tag: 'OpenClaw', status: 'ready' }
];
const COMMANDS = [
  { name: 'reply', label: 'Draft a reply', description: 'Write a reply in my voice', icon: 'pen' },
  { name: 'brief', label: 'Brief me', description: 'Who is this and what do I need to know', icon: 'layers' },
  { name: 'tasks', label: 'Plan follow-ups', description: 'Extract actions and propose tasks', icon: 'list-check' },
  { name: 'team', label: 'Tell the team', description: 'Draft a message for the right channel', icon: 'send' },
  { name: 'update', label: 'Update systems', description: 'Record this where it belongs', icon: 'globe' },
  { name: 'unsubscribe', label: 'Unsubscribe', description: 'Unsubscribe from this sender', icon: 'x' },
  { name: 'summary', label: 'Summarize', description: 'Three bullets and whether it needs a reply', icon: 'list' },
  { name: 'triage', label: 'What needs me today?', description: 'Look at the inbox across accounts', icon: 'mail' }
];
const SKILLS = [
  { name: 'meeting-prep', description: 'Prepare for a meeting: who is coming, what was said before and what to decide.', scope: 'productivity' },
  { name: 'invoice-check', description: 'Check an invoice against the quote and the contract before it is paid.', scope: 'finance' },
  { name: 'crm-update', description: 'Record a contact, deal or note in the CRM.', scope: 'sales' }
];
const APPROVAL_LABELS = {
  approved: 'Approved',
  denied: 'Denied',
  expired: 'Expired',
  waiting: 'Waiting for you',
  showAll: 'Show all',
  showLess: 'Show less',
  truncated: (n) => `${n.toLocaleString('en-GB')} more characters not shown`
};
const LONG_COMMAND = `td add "Send proposal" --due friday --project Sales --note "${'Call Sanne on Thursday at 10:00 about proposal v3. '.repeat(10)}" --priority 1`;
const STATUSES = { proposed: 'Proposed', todo: 'To do', running: 'Running', done: 'Done', failed: 'Failed', skipped: 'Skipped' };
const noop = () => {};

// ---------- column 1: atoms, text, thinking, tools ----------

const a = col('atoms');
head(a, 'Buttons');
a.append(
  row(
    bui.button({ label: 'Allow', kind: 'primary', size: 'sm' }),
    bui.button({ label: 'Show', kind: 'secondary', size: 'sm' }),
    bui.button({ label: 'Skip', kind: 'ghost', size: 'sm' }),
    bui.button({ label: 'Deny', kind: 'danger', size: 'sm' }),
    bui.button({ label: 'Undo', kind: 'quiet', size: 'xs', icon: 'undo' })
  ),
  row(
    bui.button({ label: 'Send', kind: 'primary', size: 'md', icon: 'send' }),
    bui.button({ label: 'Alternatives', kind: 'secondary', size: 'md' }),
    bui.button({ label: 'Disabled', kind: 'primary', size: 'md', disabled: true }),
    bui.button({ label: 'Set up Claude', kind: 'link', size: 'sm' })
  )
);
head(a, 'Monograms and chips');
a.append(
  row(
    bui.monogram({ label: 'Sanne de Vries' }),
    bui.monogram({ label: 'Joris Bakker', size: 20 }),
    bui.monogram({ label: 'C', size: 24, agent: true }),
    bui.monogram({ label: 'ANWB', size: 20, src: 'assets/rukoo-icon.svg' }),
    bui.entityChip({ label: 'Sanne de Vries', sub: 'Call on Thursday', monogram: 'Sanne de Vries', onRemove: noop, removeLabel: 'Remove' }),
    bui.entityChip({ label: 'Joris Bakker', monogram: { label: 'Joris Bakker' } }),
    bui.entityChip({ label: 'Thursday launch', sub: 'Meeting deleted', monogram: 'Thursday launch' }),
    bui.addChip({ label: 'Add this email', title: 'Call on Thursday', onClick: noop })
  )
);
head(a, 'Shimmer and loading');
a.append(row(bui.shimmer('Clark is thinking'), bui.loadingState({ label: 'Searching mail' })));
head(a, 'Icons');
const icons = document.createElement('div');
icons.className = 'gicons';
for (const name of [
  'plus', 'clock', 'more', 'send', 'stop', 'check', 'x', 'chevron', 'chevron-up', 'chevron-right', 'file', 'clip', 'globe', 'layers', 'chart', 'mic', 'sparkle', 'search',
  'mail', 'list', 'list-check', 'link', 'arrow-right', 'arrow-out', 'undo', 'pen', 'terminal', 'retry', 'copy', 'shield', 'wrench', 'person', 'calendar', 'info', 'bolt', 'skill',
  'sliders', 'spinner'
]) {
  const cell = document.createElement('span');
  cell.title = name;
  cell.append(bui.icon(name, 16));
  icons.append(cell);
}
a.append(icons);
head(a, 'Suggestion chips');
// With the round chip after them that leads to the shortcut settings.
const suggestions = bui.suggestionChips(
  [
    { id: 'reply', label: 'Draft a reply', icon: 'pen' },
    { id: 'brief', label: 'Brief me', icon: 'layers' },
    { id: 'tasks', label: 'Plan follow-ups', icon: 'list-check' },
    { id: 'team', label: 'Tell the team', icon: 'send' },
    { id: 'summary', label: 'Summarize', icon: 'list' }
  ],
  noop
);
suggestions.append(bui.suggestionEdit({ label: 'Edit shortcuts', index: 5, onClick: noop }));
a.append(suggestions);
head(a, 'User bubbles and system lines');
a.append(
  bui.userBubble({ text: 'Draft a reply' }),
  bui.userBubble({ text: 'Can you check whether we already agreed on a price in an earlier thread?\nAnd keep it short.' }),
  bui.userBubble({ text: 'Use $meeting-prep for Thursday, then $crm-update with what we decide.', skills: SKILLS }),
  bui.userBubble({ text: 'Add 2 tasks to Todoist', action: 'approved', labels: { approved: 'You approved', declined: 'You declined' } }),
  bui.userBubble({ text: 'Send a Slack message to #sales', action: 'declined', labels: { approved: 'You approved', declined: 'You declined' } })
);
// Messages sent while the agent worked, at the end of the conversation: steering (the agent reads it at its next
// step), queued while it works, queued because the agent can't steer, and held after Stop.
head(a, 'Waiting messages');
const queueActions = (running) => [
  { id: 'send', label: running ? 'Steer now' : 'Send now', icon: 'send', onClick: noop },
  { id: 'edit', label: 'Edit', icon: 'pen', onClick: noop },
  { id: 'remove', label: 'Remove', icon: 'x', onClick: noop }
];
a.append(
  bui.queuedBubble({ text: 'Use the work account, not the personal one.', tone: 'steering', status: 'Clark reads it before its next step' }),
  bui.queuedBubble({ text: 'Then summarize what you found in three bullets.', tone: 'queued', status: 'Sends when Clark is done', actions: queueActions(true) }),
  bui.queuedBubble({ text: 'And check the invoice too.', tone: 'queued', status: "OpenClaw can't read messages while it works. This one sends when OpenClaw is done.", actions: queueActions(false).slice(1) }),
  bui.queuedBubble({
    text: 'Compare it with this one.',
    attachments: [bui.attachmentChip({ label: 'Invoice March', sub: 'Joris Bakker', icon: 'mail' })],
    tone: 'held',
    status: 'Not sent yet',
    actions: queueActions(false)
  })
);
head(a, 'Stream text');
const doneText = bui.streamText();
doneText.set('I checked your earlier mail with Sanne. You agreed on the revised price in March, so the reply only confirms Thursday.\n\nSee https://example.com/proposal-v3 for the version she attached.');
doneText.finish();
const interim = bui.streamText({ interim: true });
interim.set('Searching for earlier threads with Sanne first.');
interim.finish();
const paused = bui.streamText();
paused.set('The draft is in the composer. I assumed you still want to call her yourself');
const markdown = bui.streamText();
markdown.set(
  'Here is the **plan** for *Thursday*:\n\n- Confirm the call at `10:00`\n- Review [the proposal](https://example.com/proposal-v3)\n  - pricing table on page 3\n- Send notes to Joris\n\n### Open question\n> Is version 3 final?\n\n```\ntd add "Send proposal" --due friday\n```'
);
markdown.finish();
a.append(doneText.el, interim.el, paused.el, markdown.el);
head(a, 'Thinking');
const thinkRun = bui.thinking({ labels: { thinking: 'Thinking', thoughtFor: (s) => `Thought for ${s}s` }, startedAt: Date.now() - 3200 });
thinkRun.update({ text: 'Reading the thread and checking what we agreed earlier. The March thread mentions a revised price, so the reply should reference it.', status: 'running' });
const thinkDone = bui.thinking({ labels: { thinking: 'Thinking', thoughtFor: (s) => `Thought for ${s}s` }, startedAt: Date.now() - 4000 });
thinkDone.update({ text: 'Reading the thread and checking what we agreed earlier.', status: 'done', endedAt: Date.now() });
const thinkEmpty = bui.thinking({ labels: { thinking: 'Thinking', thoughtFor: (s) => `Thought for ${s}s` }, startedAt: Date.now() - 1500 });
thinkEmpty.update({ text: '', status: 'done', endedAt: Date.now() });
a.append(thinkRun.el, thinkDone.el, thinkEmpty.el);
head(a, 'Tool chips');
const tools = bui.toolGroup();
tools.add(bui.toolChip({ label: 'Read the email', kind: 'mail', status: 'done', rukoo: true }));
tools.add(bui.toolChip({ label: 'Searched mail', kind: 'search', status: 'done', rukoo: true, detail: 'sanne proposal' }));
tools.add(bui.toolChip({ label: 'Wrote the draft', kind: 'draft', status: 'done', rukoo: true, detail: 'Reply to Sanne de Vries' }));
tools.add(bui.toolChip({ label: 'Showed a plan', kind: 'plan', status: 'done', rukoo: true }));
tools.add(bui.toolChip({ label: 'Showed sources', kind: 'sources', status: 'done', rukoo: true }));
tools.add(bui.toolChip({ label: 'Asked for approval', kind: 'approval', status: 'done', rukoo: true }));
tools.add(bui.toolChip({ label: 'Read an attachment', kind: 'file', status: 'running', rukoo: true, detail: 'Proposal-v3.pdf' }));
const ran = bui.toolChip({ label: 'Ran a command', kind: 'command', status: 'done', detail: 'td add "Send proposal" --due friday' });
tools.add(ran);
tools.add(bui.toolChip({ label: 'todoist: add task', kind: 'tool', status: 'error', detail: 'HTTP 401 from api.todoist.com' }));
tools.add(bui.toolChip({ label: 'Searched the web', kind: 'web', status: 'done', detail: 'vandebron contract 2026' }));
tools.add(bui.toolChip({ label: 'Read a skill', kind: 'file', status: 'done', count: 4, detail: 'email-triage\nsales-followup\ntodoist\nobsidian-notes' }));
tools.add(bui.toolChip({ label: 'Searching your mail...', kind: 'search', status: 'running', rukoo: true }));
ran.toggle(true);
a.append(tools.el);
head(a, 'Notices');
a.append(
  bui.noticeLine({ text: 'Codex started a new thread.', tone: 'info' }, { labels: { undo: 'Undo' } }).el,
  bui.noticeLine({ text: 'Archived 3 emails', tone: 'success', undo: { kind: 'move', ids: [] } }, { labels: { undo: 'Undo' }, onUndo: noop }).el,
  // Full permission: a proposal went ahead without a card.
  bui.noticeLine({ text: 'Allowed without asking: Add 2 tasks to Todoist', tone: 'info', code: 'auto-approved' }, { labels: { undo: 'Undo' } }).el,
  bui.noticeLine({ text: 'Could not reach Clark.', tone: 'error', code: 'offline' }, { labels: { undo: 'Undo' } }).el
);

// ---------- column 2: cards ----------

const b = col('cards');
head(b, 'Approval cards');
const choicesAD = [
  { id: 'allow', label: 'Allow', kind: 'primary' },
  { id: 'deny', label: 'Deny', kind: 'danger' }
];
b.append(
  bui.approvalCard(
    {
      title: 'Claude wants to run a command',
      detail: 'Creates the Todoist task with the td CLI.',
      fields: [
        { key: 'command', label: 'Command', value: 'td add "Send proposal to Sanne" --due friday --project Sales' },
        { key: 'folder', label: 'Folder', value: 'C:\\Users\\Bart\\agent-workspace' }
      ],
      choices: choicesAD,
      status: 'pending',
      source: 'claude',
      kind: 'runtime'
    },
    { labels: APPROVAL_LABELS, onChoose: noop }
  ).el,
  bui.approvalCard(
    {
      title: 'Codex wants to run a command',
      fields: [
        { key: 'command', label: 'Command', value: LONG_COMMAND },
        { key: 'reason', label: 'Reason', value: 'Adds the follow-up to Todoist.' }
      ],
      choices: [
        { id: 'accept', label: 'Allow', kind: 'primary' },
        { id: 'acceptForSession', label: 'Allow for this chat', kind: 'default' },
        { id: 'decline', label: 'Deny', kind: 'danger' }
      ],
      status: 'pending',
      kind: 'runtime'
    },
    { labels: APPROVAL_LABELS, onChoose: noop }
  ).el,
  bui.approvalCard(
    {
      title: 'Clark wants to run a command',
      detail: 'Post the summary to #sales so the team sees the new date.',
      fields: [
        { label: 'Channel', value: '#sales' },
        { label: 'Message', value: 'Sanne confirmed the call for Thursday 10:00. Message-ID <a1b2c3@example.com>', truncated: 1250 }
      ],
      choices: [
        { id: 'once', label: 'Allow once', kind: 'primary' },
        { id: 'session', label: 'Allow for this turn', kind: 'default' },
        { id: 'always', label: 'Always allow', kind: 'default' },
        { id: 'deny', label: 'Deny', kind: 'danger' }
      ],
      status: 'pending'
    },
    { labels: APPROVAL_LABELS, onChoose: noop }
  ).el,
  bui.approvalCard(
    {
      title: 'Archive 3 emails',
      fields: [
        { label: 'Vandebron', value: 'Here is another copy of your contract' },
        { label: 'Vandebron', value: 'Here is another copy of your contract' },
        { label: 'Vandebron No Reply', value: 'Your password has changed' }
      ],
      choices: [
        { id: 'approve', label: 'Archive', kind: 'primary' },
        { id: 'decline', label: 'Keep', kind: 'default' }
      ],
      status: 'approved',
      decision: 'approve',
      kind: 'mail'
    },
    { labels: APPROVAL_LABELS }
  ).el,
  bui.approvalCard({ title: 'Send a Slack message to #sales', choices: choicesAD, status: 'denied', decision: 'deny' }, { labels: APPROVAL_LABELS }).el,
  bui.approvalCard({ title: 'Codex wants to change files', detail: 'workspace/notes.md', choices: choicesAD, status: 'expired' }, { labels: APPROVAL_LABELS }).el
);
head(b, 'Plan');
const plan = bui.taskRows(
  {
    title: 'Follow-ups from Sanne',
    tasks: [
      { id: 't1', title: 'Send proposal v4 before the call', system: 'Todoist', status: 'done', url: 'https://app.todoist.com/app/task/123', due: 'Wed 8 Oct' },
      { id: 't2', title: 'Call Sanne about the proposal', system: 'Calendar', status: 'running', detail: 'Thursday 10:00, 30 minutes, phone.', owner: 'Bart', due: 'Thu 9 Oct' },
      { id: 't3', title: 'Log the deal stage in the CRM', system: 'CRM', status: 'todo' },
      { id: 't4', title: 'Share the summary with Joris', system: 'Slack', status: 'proposed', detail: 'Short message with the key points and the Message-ID.' },
      { id: 't5', title: 'Update the Linear issue', system: 'Linear', status: 'failed', detail: 'Linear returned 403: the API key has no write access.' },
      { id: 't6', title: 'Forward the contract', status: 'skipped' }
    ]
  },
  { labels: { statuses: STATUSES }, onOpen: noop }
);
b.append(plan.el);
plan.el.querySelectorAll('.bui-task')[1].querySelector('.bui-task__head').click();
head(b, 'Sources');
b.append(
  bui.contextCards(
    {
      title: 'Sources',
      sources: [
        { title: 'Re: Proposal pricing', source: 'Sanne de Vries', snippet: 'Agreed, let us go with the revised price of 4.800 for the first phase and revisit after the pilot.', messageId: 'demo:INBOX:12' },
        { title: 'Vandebron contract notes', source: 'Obsidian', snippet: 'Contract renews yearly in October. Customer number 2434702.', url: 'obsidian://open?vault=Personal&file=Vandebron' },
        { title: 'Sanne de Vries', source: 'CRM', snippet: 'Deal: Pilot phase, stage Proposal sent, owner Bart.', url: 'https://crm.example.com/contacts/sanne' },
        { title: 'Tariff changes 2026', snippet: 'Overview of the new tariffs and the dates they take effect.', url: 'https://www.vandebron.nl/tarieven' }
      ]
    },
    { onOpen: noop }
  ).el
);
head(b, 'Recommendation');
b.append(
  bui.recommendationCard(
    {
      eyebrow: 'Clark',
      icon: bui.monogram({ label: 'C', size: 14, agent: true }),
      title: 'What should Clark do with this email?',
      body: 'Sanne proposes a call on Thursday at 10:00 and attached the latest proposal.',
      actions: [
        { id: 'reply', label: 'Draft a reply', kind: 'primary' },
        { id: 'brief', label: 'Brief me', kind: 'default' }
      ]
    },
    noop
  ).el
);
// ---------- column 3: live ----------

const live = col('live');
head(live, 'Draft cards');
live.append(
  bui.draftCard(
    { mode: 'reply', to: [{ name: 'Sanne de Vries', address: 'sanne@example.com' }], subject: 'Re: Call on Thursday', summary: 'Hi Sanne, Thursday at 10:00 works for me. I will call you then. Best, Demo' },
    { labels: { title: 'Wrote a reply draft', show: 'Show', undo: 'Undo', undone: 'Undone' }, onAction: noop }
  ).el,
  bui.draftCard(
    { mode: 'new', to: [{ name: 'Joris Bakker' }], subject: 'Summary of the Vandebron thread', summary: 'Joris, here is the short version of what Sanne and I agreed.', undone: true },
    { labels: { title: 'Wrote a new draft', show: 'Show', undo: 'Undo', undone: 'Undone' }, onAction: noop }
  ).el
);
head(live, 'Composer');
const composerLabels = {
  send: 'Send',
  stop: 'Stop',
  agent: 'Agent',
  removeContext: 'Remove',
  noMatches: (q) => `No matches for "${q}"`,
  steer: 'Steer',
  queue: 'Queue',
  steerHint: (name) => `${name} reads it before its next step`,
  queueHint: (name) => `Sends when ${name} is done`,
  cantSteer: (name) => `${name} can't read messages while it works`,
  sendOptions: 'Send options'
};
const c1 = bui.chatComposer({ placeholder: 'Ask Clark about this email…', labels: composerLabels, agents: AGENTS, agentId: 'clark', commands: COMMANDS }, {});
c1.setContext(bui.entityChip({ label: 'Sanne de Vries', sub: 'Call on Thursday', monogram: 'Sanne de Vries', onRemove: noop, removeLabel: 'Remove' }));
// The permission level by Send, one per level: Ask, Auto, and Full permission with its orange shield.
const PERMISSION_ITEMS = () => [
  { heading: 'Permission level' },
  { id: '', label: 'Default', tag: 'Ask', checked: false },
  { separator: true },
  { id: 'ask', label: 'Ask', sub: 'Asks before every action', icon: 'shield' },
  { id: 'auto', label: 'Auto', sub: 'Asks before deleting or what others see', icon: 'shield-half', checked: true },
  { id: 'full', label: 'Full permission', sub: 'Never asks, not even before sending', icon: 'shield-off' }
];
c1.setPicker('permission', { text: 'Ask', label: 'Permission level: Ask', items: PERMISSION_ITEMS, icon: 'shield' });
const c2 = bui.chatComposer({ placeholder: 'Ask Claude…', labels: composerLabels, agents: AGENTS, agentId: 'claude', commands: COMMANDS }, {});
c2.setValue('Keep it short and mention the revised price from March.');
c2.setRunning(true);
// The same for an agent that can't take a message during its turn: the main button queues.
const c8 = bui.chatComposer({ placeholder: 'Ask OpenClaw…', labels: composerLabels, agents: AGENTS, agentId: 'openclaw', commands: COMMANDS }, {});
c8.setValue('And check the invoice too.');
c8.setRunning(true);
c8.setSteer(false);
// Running with nothing typed: Stop alone, filled.
const c9 = bui.chatComposer({ placeholder: 'Ask Claude…', labels: composerLabels, agents: AGENTS, agentId: 'claude', commands: COMMANDS }, {});
c9.setRunning(true);
c2.setPicker('permission', { text: 'Auto', label: 'Permission level: Auto', items: PERMISSION_ITEMS, icon: 'shield-half' });
const c3 = bui.chatComposer({ placeholder: 'Ask Codex…', labels: composerLabels, agents: AGENTS, agentId: 'codex', commands: COMMANDS }, {});
c3.setDisabled(true, bui.button({ label: 'Set up Codex', kind: 'link', size: 'sm' }));
// What goes along with the message: a file that is there (with its x), one still on its way and an email. Until the
// pending file is there the composer holds: Send is off and says why, Enter keeps the text.
const c5 = bui.chatComposer({ placeholder: 'Ask Clark…', labels: { ...composerLabels, attach: 'Attach files' }, agents: AGENTS, agentId: 'clark', commands: COMMANDS }, { onAttach: noop });
c5.setContext(bui.entityChip({ label: 'Sanne de Vries', sub: 'Call on Thursday', monogram: 'Sanne de Vries', onRemove: noop, removeLabel: 'Remove' }));
c5.setAttachments([
  bui.attachmentChip({ label: 'Proposal-v3.pdf', sub: '248 KB', icon: 'file-text', onRemove: noop, removeLabel: 'Remove Proposal-v3.pdf' }),
  bui.attachmentChip({ label: 'Floor plan.png', sub: '1.2 MB', pending: true, busy: 'Adding Floor plan.png' }),
  bui.attachmentChip({ label: 'Invoice March', sub: 'Joris Bakker', icon: 'mail', onRemove: noop, removeLabel: 'Remove Invoice March' })
]);
c5.setValue('Compare these with the offer from March.');
c5.setHolding(true, 'Wait until the files are added, then send.');
c5.setPicker('permission', { text: 'Full', label: 'Permission level: Full permission', items: PERMISSION_ITEMS, icon: 'shield-off', tone: 'warn' });
const c4wrap = document.createElement('div');
c4wrap.className = 'gpush';
const c4 = bui.chatComposer({ placeholder: 'Ask Clark…', labels: composerLabels, agents: AGENTS, agentId: 'clark', commands: COMMANDS }, {});
c4wrap.append(c4.el);
// The $ menu: the agent's own skills, with the line that says whose they are, and a group after each description.
const c6wrap = document.createElement('div');
c6wrap.className = 'gpush';
const c6 = bui.chatComposer({ placeholder: 'Ask Clark…', labels: composerLabels, agents: AGENTS, agentId: 'clark', commands: COMMANDS }, {});
c6.setSkills({ items: SKILLS, heading: "Clark's skills" });
c6wrap.append(c6.el);
// A skill in the input: the chip it becomes once a space follows its name.
const c7 = bui.chatComposer({ placeholder: 'Ask Clark…', labels: composerLabels, agents: AGENTS, agentId: 'clark', commands: COMMANDS }, {});
c7.setSkills({ items: SKILLS, heading: "Clark's skills" });
c7.setValue('Prepare the call with $meeting-prep and keep it short.');
live.append(c1.el, c2.el, c8.el, c9.el, c3.el, c5.el, c7.el, c4wrap, c6wrap);
c6.setValue('Check this with $');
c6.el.querySelector('.bui-pb__ta').dispatchEvent(new Event('input'));

const liveThread = document.createElement('div');
liveThread.style.cssText = 'display:flex;flex-direction:column;gap:10px;min-height:1180px';
head(live, 'Live');
live.append(liveThread);
const liveComposer = bui.chatComposer({ placeholder: 'Ask Clark about this email…', labels: composerLabels, agents: AGENTS, agentId: 'clark', commands: COMMANDS }, { onSend: () => run() });
liveComposer.setContext(bui.entityChip({ label: 'Sanne de Vries', sub: 'Call on Thursday', monogram: 'Sanne de Vries', onRemove: noop, removeLabel: 'Remove' }));
live.append(liveComposer.el);

let runId = 0;
async function run() {
  const id = ++runId;
  const alive = () => id === runId;
  liveThread.replaceChildren();
  liveComposer.setRunning(true);
  liveThread.append(bui.userBubble({ text: 'Draft a reply' }));
  await sleep(300);
  if (!alive()) return;
  const think = bui.thinking({ labels: { thinking: 'Thinking', thoughtFor: (s) => `Thought for ${s}s` } });
  liveThread.append(think.el);
  const thought = 'Reading the thread and checking what we agreed earlier. The March thread mentions a revised price, so the reply should reference it.';
  for (let i = 0; i < thought.length && alive(); i += 6) {
    think.update({ text: thought.slice(0, i + 6) });
    await sleep(30);
  }
  await sleep(500);
  if (!alive()) return;
  think.update({ status: 'done', endedAt: Date.now() });
  const group = bui.toolGroup();
  liveThread.append(group.el);
  const chip1 = group.add(bui.toolChip({ label: 'Read the email', kind: 'mail', status: 'running', rukoo: true }));
  await sleep(600);
  chip1.update({ status: 'done' });
  const chip2 = group.add(bui.toolChip({ label: 'Searching mail', kind: 'search', status: 'running', rukoo: true }));
  await sleep(900);
  if (!alive()) return;
  chip2.update({ label: 'Searched mail', status: 'done', detail: 'sanne proposal' });
  const stream = bui.streamText();
  liveThread.append(stream.el);
  const answer =
    'I checked your earlier mail with Sanne. You agreed on the revised price in March, so the reply only confirms Thursday at 10:00 and thanks her for the new version.\n\nI kept it to three sentences.';
  for (let i = 0; i < answer.length && alive(); i += 6) {
    stream.append(answer.slice(i, i + 6));
    await sleep(30);
  }
  if (!alive()) return;
  stream.finish();
  const chip3 = group.add(bui.toolChip({ label: 'Writing the draft', kind: 'draft', status: 'running', rukoo: true }));
  await sleep(800);
  if (!alive()) return;
  chip3.update({ label: 'Wrote the draft', status: 'done', detail: 'Reply to Sanne de Vries' });
  liveThread.append(
    bui.draftCard(
      { mode: 'reply', to: [{ name: 'Sanne de Vries' }], subject: 'Re: Call on Thursday', summary: 'Hi Sanne, Thursday at 10:00 works for me. I will call you then.' },
      { labels: { title: 'Wrote a reply draft', show: 'Show', undo: 'Undo', undone: 'Undone' }, onAction: noop }
    ).el
  );
  const tail = bui.streamText();
  liveThread.append(tail.el);
  const tailText = 'Done. The draft is in the composer. I assumed you still want to call her yourself.';
  for (let i = 0; i < tailText.length && alive(); i += 6) {
    tail.append(tailText.slice(i, i + 6));
    await sleep(30);
  }
  if (!alive()) return;
  tail.finish();
  await sleep(300);
  const approval = bui.approvalCard(
    {
      title: 'Archive 2 emails',
      fields: [
        { label: 'Vandebron', value: 'Here is another copy of your contract' },
        { label: 'Vandebron', value: 'Here is another copy of your contract' }
      ],
      choices: [
        { id: 'approve', label: 'Archive', kind: 'primary' },
        { id: 'decline', label: 'Keep', kind: 'default' }
      ],
      status: 'pending',
      kind: 'mail'
    },
    {
      labels: APPROVAL_LABELS,
      onChoose: (choice) => {
        approval.update({ status: choice === 'approve' ? 'approved' : 'denied', decision: choice });
      }
    }
  );
  liveThread.append(approval.el);
  liveComposer.setRunning(false);
  await sleep(2200);
  if (!alive()) return;
  approval.update({ status: 'approved', decision: 'approve' });
  await sleep(400);
  if (!alive()) return;
  liveThread.append(bui.noticeLine({ text: 'Archived 2 emails', tone: 'success', undo: { kind: 'move', ids: [] } }, { labels: { undo: 'Undo' }, onUndo: noop }).el);
}

// ---------- behaviour checks (node scripts/agent-gallery.js --check) ----------

// Runs in the real renderer so keyboard rules, menus and streaming are verified where they ship.
async function check() {
  const fails = [];
  const ok = (cond, msg) => {
    if (!cond) fails.push(msg);
  };
  const key = (el, k, init = {}) => el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }));
  const input = (el) => el.dispatchEvent(new Event('input', { bubbles: true }));
  const scratch = document.createElement('div');
  scratch.className = 'bui';
  scratch.style.cssText = 'position:fixed;left:0;top:0;width:380px;opacity:0;pointer-events:none';
  document.body.append(scratch);

  // streamText: paragraphs, line breaks, links, caret, escaping, prefix appends and resets.
  const st = bui.streamText();
  scratch.append(st.el);
  st.set('Hello <b>world</b>.\n\nSee https://example.com/x. Line\nbreak');
  await sleep(700);
  st.finish();
  await sleep(60);
  ok(st.el.querySelectorAll('p').length === 2, 'streamText splits paragraphs on blank lines');
  ok(st.el.querySelector('a')?.getAttribute('href') === 'https://example.com/x', 'streamText links URLs without the trailing dot');
  ok(st.el.querySelector('br'), 'streamText turns single newlines into <br>');
  ok(!st.el.querySelector('b') && st.el.textContent.includes('<b>world</b>'), 'streamText never interprets agent text as HTML');
  ok(!st.el.querySelector('.bui-stream__caret'), 'streamText drops the caret after finish');
  ok(!st.streaming, 'streamText reports not streaming when done');
  const st2 = bui.streamText();
  scratch.append(st2.el);
  st2.append('abc');
  st2.append('def');
  await sleep(150);
  ok(st2.el.textContent === 'abcdef', 'streamText append renders deltas');
  ok(st2.streaming && st2.el.querySelector('.bui-stream__caret.is-streaming'), 'streamText keeps a solid caret while the turn is open');
  st2.set('abcdefgh');
  await sleep(100);
  ok(st2.el.textContent === 'abcdefgh', 'streamText set() with a longer prefix continues');
  st2.set('zzz');
  await sleep(100);
  ok(st2.el.textContent === 'zzz', 'streamText set() with other text resets');
  st2.finish();

  // Markdown: the subset agents write, built as DOM; unsafe links stay plain text.
  const md = bui.streamText();
  scratch.append(md.el);
  md.set(
    'A **bold** and *italic* and `code` and _under_ but snake_case_name stays.\nNext line.\n\n' +
      '- one\n- two\n  - nested\n\n3. three\n4. four\n\n### Heading\n> quoted\n\n' +
      '```\nx = 1\n\ny = 2\n```\n\n' +
      '[safe](https://example.com/a) [bad](javascript:alert(1)) mailto:me@example.com <img src=x onerror=alert(1)>'
  );
  md.finish();
  await sleep(30);
  const q = (s) => md.el.querySelector(s);
  ok(q('strong')?.textContent === 'bold' && q('em')?.textContent === 'italic' && q('p code')?.textContent === 'code', 'markdown renders bold, italic and code spans');
  ok([...md.el.querySelectorAll('em')].some((e) => e.textContent === 'under') && md.el.textContent.includes('snake_case_name'), 'markdown italic underscores only at word edges');
  ok(md.el.querySelector('p br'), 'markdown keeps single line breaks');
  ok(md.el.querySelectorAll('ul > li').length === 3 && q('ul li ul li')?.textContent === 'nested', 'markdown renders a bullet list with a nested item');
  ok(q('ol')?.getAttribute('start') === '3' && md.el.querySelectorAll('ol > li').length === 2, 'markdown keeps the number an ordered list starts at');
  ok(q('p.bui-md-h')?.textContent === 'Heading' && q('blockquote')?.textContent.trim() === 'quoted', 'markdown renders headings as bold lines and quotes');
  ok(q('pre code')?.textContent === 'x = 1\n\ny = 2', 'markdown keeps a fenced code block whole, blank lines included');
  const links = [...md.el.querySelectorAll('a')].map((l) => l.getAttribute('href'));
  ok(links.join(' ') === 'https://example.com/a mailto:me@example.com', 'markdown links only http, https and mailto');
  ok(md.el.textContent.includes('bad') && !q('img') && md.el.textContent.includes('<img src=x onerror=alert(1)>'), 'markdown never turns agent text into HTML');
  const mdLive = bui.streamText();
  scratch.append(mdLive.el);
  mdLive.append('Here is **the pl');
  await sleep(200);
  ok(mdLive.el.querySelector('strong') && !mdLive.el.textContent.includes('**'), 'while streaming an unclosed ** formats instead of showing the marker');
  ok(mdLive.el.querySelector('.bui-stream__caret.is-streaming'), 'markdown streaming keeps the caret');
  mdLive.append('an**.\n\n- a\n- b');
  await sleep(200);
  ok(mdLive.el.querySelector('ul li:last-child .bui-stream__caret'), 'the caret sits at the end of the last list item');
  mdLive.finish();
  await sleep(30);
  ok(!mdLive.el.querySelector('.bui-stream__caret') && mdLive.el.querySelector('strong')?.textContent === 'the plan', 'finish renders the final text once, without the caret');

  // thinking: auto-expand while running with text, collapse when done, manual toggle wins.
  const th = bui.thinking({ labels: { thinking: 'T', thoughtFor: (s) => `for ${s}` }, startedAt: Date.now() - 2500 });
  scratch.append(th.el);
  ok(!th.el.classList.contains('is-expanded') && th.el.querySelector('.bui-think__head').disabled, 'thinking without text is not expandable');
  th.update({ text: 'abc' });
  ok(th.el.classList.contains('is-expanded'), 'thinking auto-expands while running with text');
  th.update({ status: 'done', endedAt: Date.now() });
  ok(!th.el.classList.contains('is-expanded'), 'thinking collapses when done');
  ok(th.el.querySelector('.bui-think__done').textContent === 'for 3', 'thinking shows the thought-for label with rounded seconds');
  th.el.querySelector('.bui-think__head').click();
  ok(th.el.classList.contains('is-expanded'), 'thinking head click expands');

  // tool chips and group: status classes, detail toggling, one open detail per group.
  const group = bui.toolGroup();
  scratch.append(group.el);
  const chipA = group.add(bui.toolChip({ label: 'A', kind: 'search', status: 'running', detail: 'q' }));
  const chipB = group.add(bui.toolChip({ label: 'B', kind: 'command', status: 'done', detail: 'ls' }));
  ok(chipA.el.classList.contains('is-running') && chipA.el.querySelector('.bui-spin'), 'toolChip running shows a spinner');
  chipA.update({ status: 'error' });
  ok(chipA.el.classList.contains('is-error') && !chipA.el.querySelector('.bui-spin'), 'toolChip update to error swaps the mark');
  chipA.el.click();
  ok(!chipA.detail.hidden, 'toolChip click opens its detail');
  chipB.el.click();
  ok(chipA.detail.hidden && !chipB.detail.hidden, 'toolGroup keeps one detail open');
  ok(chipB.detail.classList.contains('is-mono'), 'command detail is monospaced');
  const chipC = bui.toolChip({ label: 'Read a skill', kind: 'file', status: 'done', count: 3, detail: 'a\nb\nc' });
  ok(chipC.el.querySelector('.bui-tool__count')?.textContent === '×3', 'toolChip shows the count of calls it stands for');
  chipC.update({ count: 1 });
  ok(chipC.el.querySelector('.bui-tool__count').hidden, 'toolChip hides a count of one');
  group.set([chipB, chipC]);
  ok([...group.el.querySelectorAll('.bui-tools__row > .bui-tool')].map((el) => el.textContent).join(',') === 'B,Read a skill' && !chipA.el.isConnected, 'toolGroup.set replaces the row in order');
  ok(!chipB.detail.hidden, 'toolGroup.set keeps an open chip open');

  // approval card: choices disabled after choosing, decision rendering.
  let chosen = null;
  const ap = bui.approvalCard(
    { title: 'T', choices: [{ id: 'allow', label: 'Allow', kind: 'primary' }, { id: 'deny', label: 'Deny', kind: 'danger' }], status: 'pending' },
    { labels: APPROVAL_LABELS, onChoose: (id) => (chosen = id) }
  );
  scratch.append(ap.el);
  const buttons = [...ap.el.querySelectorAll('.bui-approval__actions button')];
  ok(buttons.map((b) => b.textContent).join(',') === 'Deny,Allow', 'approval puts the primary choice last');
  buttons[1].click();
  ok(chosen === 'allow' && buttons.every((b) => b.disabled), 'approval choose reports the id and disables the buttons');
  ap.update({ status: 'approved', decision: 'allow' });
  ok(ap.el.querySelector('.bui-donepill')?.textContent === 'Approved' && !ap.el.querySelector('.bui-approval__actions button'), 'approval renders the approved pill');
  ok(!ap.el.textContent.includes('null'), 'approval never prints null');
  // Long values: the whole value is there, folded until "Show all"; a value main shortened says so.
  const longValue = 'x'.repeat(850) + ' END';
  const ap2 = bui.approvalCard(
    { title: 'T', fields: [{ key: 'command', label: 'Command', value: longValue, truncated: 1234 }, { label: 'Channel', value: '#sales' }], choices: choicesAD, status: 'pending' },
    { labels: APPROVAL_LABELS }
  );
  scratch.append(ap2.el);
  const v = ap2.el.querySelector('.bui-approval__v.is-code');
  ok(v && v.querySelector('.bui-approval__text').textContent === longValue, 'approval keeps the full value of a field');
  ok(v.classList.contains('is-folded') && v.querySelector('.bui-approval__more')?.textContent === 'Show all', 'approval folds a long value behind Show all');
  ok(v.querySelector('.bui-approval__text').getBoundingClientRect().height < 160, 'a folded value takes about six lines');
  v.querySelector('.bui-approval__more').click();
  ok(!v.classList.contains('is-folded') && v.querySelector('.bui-approval__more').textContent === 'Show less', 'Show all unfolds the value');
  ap2.update({ detail: 'again' });
  ok(!ap2.el.querySelector('.bui-approval__v.is-code').classList.contains('is-folded'), 'an unfolded value stays open when the card is drawn again');
  ok(ap2.el.querySelector('.bui-approval__cut')?.textContent === '1,234 more characters not shown', 'approval says how much main left out');
  ok(!ap2.el.querySelectorAll('.bui-approval__v')[1].classList.contains('is-code') && !ap2.el.querySelectorAll('.bui-approval__v')[1].querySelector('.bui-approval__more'), 'short plain fields stay plain');
  const ap4 = bui.approvalCard(
    {
      title: 'T',
      choices: [
        { id: 'once', label: 'Allow once', kind: 'primary' },
        { id: 'session', label: 'Allow for this turn', kind: 'default' },
        { id: 'always', label: 'Always allow', kind: 'default' },
        { id: 'deny', label: 'Deny', kind: 'danger' }
      ],
      status: 'pending'
    },
    { labels: APPROVAL_LABELS }
  );
  scratch.append(ap4.el);
  const fourButtons = [...ap4.el.querySelectorAll('.bui-approval__actions button')].map((b) => b.getBoundingClientRect());
  ok(ap4.el.querySelector('.bui-approval__actions.is-many') && fourButtons[2].top === fourButtons[3].top && fourButtons[0].top === fourButtons[1].top, 'four choices sit two by two, the primary never alone');

  // task rows: reconcile by id, keep order, drop removed rows.
  const tr = bui.taskRows({ title: 'P', tasks: [{ id: 'a', title: 'A', status: 'todo' }, { id: 'b', title: 'B', status: 'running' }] }, { labels: { statuses: STATUSES } });
  scratch.append(tr.el);
  const firstRow = tr.el.querySelector('.bui-task');
  tr.update({ title: 'P', tasks: [{ id: 'b', title: 'B2', status: 'done', url: 'https://x.test/1' }, { id: 'c', title: 'C', status: 'failed' }] });
  const rowsNow = [...tr.el.querySelectorAll('.bui-task')];
  ok(rowsNow.length === 2 && !rowsNow.includes(firstRow), 'taskRows removes missing tasks');
  ok(rowsNow[0].querySelector('.bui-task__label').textContent === 'B2' && rowsNow[0].querySelector('.bui-task__pill--done'), 'taskRows updates a row in place');
  ok(tr.el.querySelector('.bui-count').textContent === '2', 'taskRows count follows the tasks');
  let opened = null;
  const tr2 = bui.taskRows({ title: 'P', tasks: [{ id: 'a', title: 'A', status: 'done', url: 'https://x.test/1' }] }, { labels: { statuses: STATUSES }, onOpen: (u) => (opened = u) });
  scratch.append(tr2.el);
  tr2.el.querySelector('.bui-task__head').click();
  ok(tr2.el.querySelector('.bui-task').classList.contains('is-open'), 'taskRows head click opens the drawer');
  tr2.el.querySelector('.bui-task__link').click();
  ok(opened === 'https://x.test/1', 'taskRows link calls onOpen with the url');

  // context cards, draft card, notice, user bubble.
  let src = null;
  const cc = bui.contextCards({ title: 'S', sources: [{ title: 'M', messageId: 'id1' }, { title: 'N', snippet: 'x' }] }, { onOpen: (s) => (src = s) });
  scratch.append(cc.el);
  cc.el.querySelector('.bui-ctx__chip').click();
  ok(src && src.messageId === 'id1', 'contextCards chip opens the source');
  ok(cc.el.querySelectorAll('.bui-ctx__card').length === 2 && cc.el.querySelectorAll('button.bui-ctx__chip').length === 1, 'contextCards only makes targets clickable');
  let drafted = null;
  const dc = bui.draftCard({ to: [{ name: 'S' }], subject: 'Re', summary: 'x' }, { labels: { title: 'W', show: 'Show', undo: 'Undo', undone: 'Undone' }, onAction: (a) => (drafted = a) });
  scratch.append(dc.el);
  dc.el.querySelectorAll('.bui-btn')[1].click();
  ok(drafted === 'undo', 'draftCard undo action');
  dc.update({ canUndo: false });
  ok(dc.el.querySelectorAll('.bui-btn')[1].disabled, 'draftCard turns Undo off when its composer is gone');
  dc.update({ canUndo: true });
  ok(!dc.el.querySelectorAll('.bui-btn')[1].disabled, 'draftCard turns Undo back on');
  dc.update({ undone: true });
  ok(dc.el.classList.contains('is-undone') && dc.el.querySelector('.bui-draft__undone'), 'draftCard renders the undone state');
  let undone = false;
  const nl = bui.noticeLine({ text: 'n', tone: 'success', undo: { kind: 'move', ids: [] } }, { labels: { undo: 'Undo' }, onUndo: () => (undone = true) });
  scratch.append(nl.el);
  nl.el.querySelector('.bui-notice__undo').click();
  ok(undone, 'noticeLine undo calls back');
  ok(bui.userBubble({ text: 'T', action: 'approved', labels: { approved: 'You approved' } }).classList.contains('bui-sys'), 'userBubble approved renders a system line');
  ok(bui.userBubble({ text: '<i>x</i>' }).textContent === '<i>x</i>', 'userBubble escapes text');
  const picked = [];
  const qb = bui.queuedBubble({ text: '<b>x</b>', status: 'Sends when Clark is done', actions: [{ id: 'edit', label: 'Edit', icon: 'pen', onClick: () => picked.push('edit') }] });
  ok(qb.querySelector('.bui-ub').textContent === '<b>x</b>' && !qb.querySelector('b'), 'queuedBubble escapes text');
  qb.querySelector('[data-action="edit"]').click();
  ok(picked[0] === 'edit' && qb.querySelector('[data-action="edit"]').disabled && qb.querySelector('[data-action="edit"]').getAttribute('aria-label') === 'Edit', 'a waiting message’s action calls back once and is labelled');
  const steeringBubble = bui.queuedBubble({ text: 'x', tone: 'steering', status: 'Clark reads it before its next step' });
  ok(steeringBubble.querySelector('.bui-spin') && !steeringBubble.querySelector('button'), 'a steering message shows a spinner and no actions');

  // chat composer: Enter sends, Shift+Enter and IME do not, stop while running, slash menu keys, disabled state.
  const got = { sent: [], stopped: 0, commands: [], agents: [] };
  const cp = bui.chatComposer(
    { placeholder: 'p', labels: composerLabels, agents: AGENTS, agentId: 'clark', commands: COMMANDS },
    { onSend: (t) => got.sent.push(t), onStop: () => got.stopped++, onCommand: (n) => got.commands.push(n), onAgent: (id) => got.agents.push(id) }
  );
  scratch.append(cp.el);
  const ta = cp.el.querySelector('.bui-pb__ta');
  const sendBtn = cp.el.querySelector('.bui-pb__send');
  ok(sendBtn.disabled, 'composer send is disabled when empty');
  cp.setValue(' hello ');
  ok(!sendBtn.disabled, 'composer send enables with text');
  key(ta, 'Enter', { shiftKey: true });
  ok(got.sent.length === 0, 'Shift+Enter does not send');
  key(ta, 'Enter', { isComposing: true });
  ok(got.sent.length === 0, 'Enter during IME composition does not send');
  key(ta, 'Enter');
  ok(got.sent[0] === 'hello' && cp.getValue() === '', 'Enter sends the trimmed text and clears');
  cp.setRunning(true);
  ok(!sendBtn.disabled && sendBtn.querySelector('[data-icon="stop"]'), 'running shows an enabled stop button');
  sendBtn.click();
  ok(got.stopped === 1, 'stop button calls onStop');
  cp.setRunning(false);

  // While the agent works: Stop stays put; a message typed meanwhile gets Steer beside it. Enter steers, Alt+Enter
  // queues, and for an agent that can't steer Enter queues and the menu says why Steer is off.
  const run = { sent: [], modes: [], stopped: 0 };
  const rc = bui.chatComposer({ placeholder: 'p', labels: composerLabels, agents: AGENTS, agentId: 'claude', commands: COMMANDS }, { onSend: (t, mode) => (run.sent.push(t), run.modes.push(mode)), onStop: () => run.stopped++ });
  scratch.append(rc.el);
  const rta = rc.el.querySelector('.bui-pb__ta');
  const rstop = rc.el.querySelector('.bui-pb__send');
  const rsplit = rc.el.querySelector('.bui-pb__split');
  rc.setValue('idle');
  key(rta, 'Enter', { altKey: true });
  ok(run.modes[0] === null, 'idle, Alt+Enter just sends');
  rc.setRunning(true);
  ok(rsplit.hidden && !rstop.classList.contains('is-quiet'), 'running with nothing typed: Stop alone');
  rc.setValue('use the work account');
  ok(!rsplit.hidden && rstop.classList.contains('is-quiet') && rstop.querySelector('[data-icon="stop"]'), 'a message typed while running: Steer beside a quieter Stop');
  ok(rsplit.querySelector('.bui-pb__go').getAttribute('aria-label') === 'Steer' && /reads it before its next step \(Enter\)/.test(rsplit.querySelector('.bui-pb__go').title), 'the main part steers and says how');
  key(rta, 'Enter');
  ok(run.sent[1] === 'use the work account' && run.modes[1] === 'steer' && rc.getValue() === '', 'Enter steers while running');
  ok(rsplit.hidden, 'with the input empty again, Stop stands alone');
  rc.setValue('then summarize');
  key(rta, 'Enter', { altKey: true });
  ok(run.modes[2] === 'queue', 'Alt+Enter queues while running');
  rc.setValue('and the invoice');
  rsplit.querySelector('.bui-pb__more').click();
  let sendMenu = document.querySelector('.bui-menu');
  ok(sendMenu && sendMenu.querySelectorAll('.bui-menu__row').length === 2 && !sendMenu.querySelector('[data-id="steer"]').disabled, 'the send menu offers Steer and Queue');
  ok(/Alt\+Enter/.test(sendMenu.querySelector('[data-id="queue"]').textContent), 'the menu names the key for Queue');
  sendMenu.querySelector('[data-id="queue"]').click();
  ok(run.modes[3] === 'queue' && !document.querySelector('.bui-menu'), 'picking Queue queues the message');
  rc.setSteer(false);
  rc.setValue('one more');
  ok(rsplit.querySelector('.bui-pb__go').getAttribute('aria-label') === 'Queue' && rsplit.querySelector('[data-icon="clock"]'), 'for an agent that can’t steer the main part queues');
  rsplit.querySelector('.bui-pb__more').click();
  sendMenu = document.querySelector('.bui-menu');
  ok(sendMenu.querySelector('[data-id="steer"]').disabled && /can't read messages while it works/.test(sendMenu.querySelector('[data-id="steer"]').textContent), 'Steer is off in the menu, and says why');
  sendMenu.querySelector('[data-id="queue"]').click();
  rc.setValue('again');
  key(rta, 'Enter');
  ok(run.modes[5] === 'queue', 'Enter queues for an agent that can’t steer');
  rstop.click();
  ok(run.stopped === 1, 'Stop still stops, whatever is typed');
  rc.el.remove();
  cp.setValue('/ta');
  input(ta);
  ok(cp.el.querySelectorAll('.bui-pb__cmd').length === 1 && cp.el.querySelector('.bui-pb__cmdname').textContent === '/tasks', 'slash menu filters commands');
  key(ta, 'Enter', { isComposing: true });
  key(ta, 'Tab', { isComposing: true });
  key(ta, 'Enter', { keyCode: 229 });
  ok(got.commands.length === 0 && cp.getValue() === '/ta' && cp.el.querySelector('.bui-pb__menu'), 'Enter and Tab during IME composition pick nothing from the slash menu');
  key(ta, 'Enter');
  ok(got.commands[0] === 'tasks' && cp.getValue() === '' && !cp.el.querySelector('.bui-pb__menu'), 'Enter picks the command, strips the token and sends nothing');
  ok(got.sent.length === 1, 'picking a command does not send text');
  cp.setValue('ask /');
  input(ta);
  ok(cp.el.querySelectorAll('.bui-pb__cmd').length === COMMANDS.length, 'slash at a word start lists every command');
  key(ta, 'ArrowDown');
  key(ta, 'ArrowDown');
  key(ta, 'Tab');
  ok(got.commands[1] === COMMANDS[2].name && cp.getValue() === 'ask ', 'arrows move the active row and Tab picks it');
  cp.setValue('a/b');
  input(ta);
  ok(!cp.el.querySelector('.bui-pb__menu'), 'a slash inside a word opens nothing');
  cp.setValue('/zzz');
  input(ta);
  ok(cp.el.querySelector('.bui-pb__empty')?.textContent === 'No matches for "zzz"', 'slash menu shows the no-matches label');
  key(ta, 'Escape');
  ok(!cp.el.querySelector('.bui-pb__menu'), 'Escape closes the slash menu');
  key(ta, 'Enter');
  ok(got.sent[1] === '/zzz', 'Enter with a dismissed menu sends the text');
  cp.setDisabled(true, 'off');
  ok(ta.hidden && cp.el.querySelector('.bui-pb__disabled').textContent === 'off' && sendBtn.disabled, 'disabled composer hides the textarea and shows the reason');
  cp.setDisabled(false);
  ok(!ta.hidden, 'enabling the composer restores the textarea');
  const trigger = cp.el.querySelector('.bui-pb__modelbtn');
  trigger.click();
  const menu = document.querySelector('.bui-menu');
  ok(menu && menu.querySelectorAll('.bui-menu__row').length === AGENTS.length && trigger.getAttribute('aria-expanded') === 'true', 'agent picker opens on the body with every agent');
  ok(document.activeElement === menu.querySelector('.bui-menu__row.is-checked'), 'agent picker focuses the current agent');
  key(document.activeElement, 'ArrowDown');
  ok(document.activeElement === menu.querySelectorAll('.bui-menu__row')[1], 'ArrowDown moves focus in the picker');
  document.activeElement.click();
  ok(got.agents[0] === 'claude' && !document.querySelector('.bui-menu'), 'picking an agent reports the id and closes');
  trigger.click();
  key(document.activeElement, 'Escape');
  ok(!document.querySelector('.bui-menu') && document.activeElement === trigger, 'Escape closes the picker and refocuses the trigger');
  trigger.click();
  document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
  ok(!document.querySelector('.bui-menu'), 'outside pointerdown closes the picker');

  // The chat list under the bell: two-line rows with a status mark and the time on the first line. update() redraws
  // an open menu, and the row with the focus keeps it.
  const bell = document.createElement('button');
  document.body.append(bell);
  const chats = (doing) => [
    { id: 'a', label: 'Call on Thursday', tag: '10:42', sub: `Claude · ${doing}`, status: doing === 'Working…' ? 'running' : 'unread' },
    { id: 'b', label: 'Your parcel is on its way', tag: '10:40', sub: 'Hermes · Waiting for you', status: 'waiting' }
  ];
  const pickedChats = [];
  const chatList = bui.glideMenu({ items: chats('Working…'), anchor: bell, onPick: (id) => pickedChats.push(id) });
  const chatRow = (id) => chatList.el.querySelector(`.bui-menu__row[data-id="${id}"]`);
  ok(
    chatRow('a').classList.contains('bui-menu__row--two') &&
      chatRow('a').querySelector('.bui-menu__mark .bui-spin') &&
      chatRow('a').querySelector('.bui-menu__top .bui-menu__tag')?.textContent === '10:42' &&
      chatRow('a').querySelector('.bui-menu__sub')?.textContent === 'Claude · Working…',
    'a two-line row has its mark, the time on its first line and its second line'
  );
  ok(chatRow('b').querySelector('.bui-dot--busy'), 'a chat waiting for approval has an orange dot');
  key(document.activeElement, 'ArrowDown');
  ok(document.activeElement === chatRow('b'), 'ArrowDown moves to the second chat');
  chatList.update(chats('Reply ready'));
  ok(chatRow('a').querySelector('.bui-dot--unread') && chatRow('a').querySelector('.bui-menu__sub').textContent === 'Claude · Reply ready', 'update() redraws the rows of the open menu');
  ok(document.activeElement === chatRow('b'), 'the row with the focus keeps it after update()');
  chatRow('a').click();
  ok(pickedChats[0] === 'a' && !document.querySelector('.bui-menu'), 'picking a chat reports its id and closes the list');
  chatList.update(chats('Working…'));
  ok(!document.querySelector('.bui-menu'), 'update() on a closed menu draws nothing');
  bell.remove();

  // attachments: a ready one has its x; a pending one has none, is busy and says what is being added. A holding
  // composer sends nothing, starts no command and keeps the text; Send comes back once it stops holding.
  const removed = [];
  const ready = bui.attachmentChip({ label: 'a.pdf', sub: '1 KB', icon: 'file-text', onRemove: () => removed.push('a.pdf'), removeLabel: 'Remove a.pdf' });
  const pend = bui.attachmentChip({ label: 'b.png', sub: '2 KB', pending: true, busy: 'Adding b.png', onRemove: () => removed.push('b.png'), removeLabel: 'Remove b.png' });
  ok(ready.querySelector('.bui-entity__x')?.getAttribute('aria-label') === 'Remove a.pdf' && !ready.hasAttribute('aria-busy') && !ready.classList.contains('is-pending'), 'a ready attachment has its remove button');
  ok(pend.classList.contains('is-pending') && !pend.querySelector('.bui-entity__x'), 'a pending attachment has no remove button');
  ok(pend.getAttribute('aria-busy') === 'true' && pend.getAttribute('aria-label') === 'Adding b.png', 'a pending attachment is busy and says what is being added');
  const held = { sent: [], commands: [] };
  const hc = bui.chatComposer(
    { placeholder: 'p', labels: { ...composerLabels, attach: 'Attach' }, agents: AGENTS, agentId: 'clark', commands: COMMANDS },
    { onSend: (t) => held.sent.push(t), onCommand: (n) => held.commands.push(n), onAttach: () => {} }
  );
  scratch.append(hc.el);
  hc.setAttachments([ready, pend]);
  ok(getComputedStyle(pend.querySelector('.bui-entity__name')).animationName === 'bui-shimmer-text', 'a pending attachment name shimmers');
  ok(getComputedStyle(ready.querySelector('.bui-entity__name')).animationName === 'none', 'a ready attachment name is still');
  const hta = hc.el.querySelector('.bui-pb__ta');
  const hsend = hc.el.querySelector('.bui-pb__send');
  hc.setValue('keep me');
  hc.setHolding(true, 'Wait for the files');
  ok(hsend.disabled && hsend.title === 'Wait for the files' && hsend.getAttribute('aria-description') === 'Wait for the files', 'a holding composer turns Send off and says why');
  key(hta, 'Enter');
  hsend.click();
  ok(held.sent.length === 0 && hc.getValue() === 'keep me', 'Enter or Send while holding sends nothing and keeps the text');
  hc.setValue('/ta');
  input(hta);
  ok(!hc.el.querySelector('.bui-pb__menu'), 'a holding composer opens no slash menu');
  key(hta, 'Enter');
  ok(held.commands.length === 0 && held.sent.length === 0 && hc.getValue() === '/ta', 'no command starts while holding');
  hc.setValue('keep me');
  hc.setHolding(false);
  ok(!hsend.disabled && !hsend.title && !hsend.hasAttribute('aria-description'), 'Send comes back when the composer stops holding');
  key(hta, 'Enter');
  ok(held.sent[0] === 'keep me' && hc.getValue() === '', 'then Enter sends the text');
  // A message its owner refuses (onSend returns false) keeps its text, by Enter and by the Send button.
  const refused = bui.chatComposer({ placeholder: 'p', labels: composerLabels, agents: AGENTS, agentId: 'clark', commands: COMMANDS }, { onSend: () => false });
  scratch.append(refused.el);
  refused.setValue('not yet');
  key(refused.el.querySelector('.bui-pb__ta'), 'Enter');
  refused.el.querySelector('.bui-pb__send').click();
  ok(refused.getValue() === 'not yet', 'a message the owner refuses keeps its text');
  // A command its owner refuses (onCommand returns false) keeps its token in the input.
  const declined = [];
  const nocmd = bui.chatComposer({ placeholder: 'p', labels: composerLabels, agents: AGENTS, agentId: 'clark', commands: COMMANDS }, { onCommand: (n) => declined.push(n) && false });
  scratch.append(nocmd.el);
  const nta = nocmd.el.querySelector('.bui-pb__ta');
  nocmd.setValue('/ta');
  input(nta);
  key(nta, 'Enter');
  ok(declined.length === 1 && nocmd.getValue() === '/ta' && !nocmd.el.querySelector('.bui-pb__menu'), 'a command the owner refuses keeps its token');

  // $ menu: the agent's own skills. Opening it asks the owner for the list once; while it loads it says so, a list
  // that failed says why. Picking a skill puts $name in the text and starts nothing; an amount opens nothing.
  const dollar = { opened: 0, commands: [], sent: [] };
  const sk = bui.chatComposer(
    { placeholder: 'p', labels: { ...composerLabels, skillsLoading: 'Loading skills…', noSkills: 'No skills found' }, agents: AGENTS, agentId: 'clark', commands: COMMANDS },
    { onDollar: () => dollar.opened++, onCommand: (n) => dollar.commands.push(n), onSend: (t) => dollar.sent.push(t) }
  );
  scratch.append(sk.el);
  const sta = sk.el.querySelector('.bui-pb__ta');
  sk.setSkills({ loading: true, heading: "Clark's skills" });
  sk.setValue('$');
  input(sta);
  ok(dollar.opened === 1 && sk.el.querySelector('.bui-pb__empty[role="status"]')?.textContent === 'Loading skills…', '$ asks for the skills and says they are loading');
  sk.setSkills({ items: SKILLS, heading: "Clark's skills" });
  ok(sk.el.querySelector('.bui-pb__heading')?.textContent === "Clark's skills" && sk.el.querySelectorAll('.bui-pb__cmd').length === 3, 'the list replaces the loading line in the open menu');
  ok([...sk.el.querySelectorAll('.bui-pb__cmdname')].map((n) => n.textContent).join() === '$meeting-prep,$invoice-check,$crm-update', 'skills show as $name');
  sk.setValue('Please $inv');
  input(sta);
  ok(dollar.opened === 1 && sk.el.querySelectorAll('.bui-pb__cmd').length === 1, 'typing narrows the skills without asking again');
  sk.setValue('$contract');
  input(sta);
  ok(sk.el.querySelector('.bui-pb__cmdname')?.textContent === '$invoice-check', 'a description matches too');
  // A list that comes in while the menu is open keeps the row the user is on, or the nearest one that is left.
  sk.setValue('$');
  input(sta);
  key(sta, 'ArrowDown');
  key(sta, 'ArrowDown');
  sk.setSkills({ items: [SKILLS[2], SKILLS[0]], heading: "Clark's skills" });
  ok(sk.el.querySelectorAll('.bui-pb__cmd')[0].getAttribute('aria-selected') === 'true', 'a new list keeps the row the user is on');
  sk.setSkills({ items: [SKILLS[0]], heading: "Clark's skills" });
  key(sta, 'Tab');
  ok(sk.getValue() === '$meeting-prep ', 'a shorter list moves the row onto one that is left, and Tab picks it');
  sk.setSkills({ items: SKILLS, heading: "Clark's skills" });
  sk.setValue('Please $inv');
  input(sta);
  key(sta, 'Enter');
  ok(sk.getValue() === 'Please $invoice-check ' && !sk.el.querySelector('.bui-pb__menu'), 'Enter puts $name in the text and closes the menu');
  ok(dollar.commands.length === 0 && dollar.sent.length === 0, 'picking a skill neither runs a command nor sends');
  sk.setValue('It costs $5');
  input(sta);
  ok(!sk.el.querySelector('.bui-pb__menu'), 'an amount opens no menu');
  sk.setSkills({ items: [], error: "Clark isn't answering" });
  sk.setValue('$');
  input(sta);
  ok(sk.el.querySelector('.bui-pb__empty')?.textContent === "Clark isn't answering", 'a list that failed says why');
  sk.setSkills({ items: [] });
  ok(sk.el.querySelector('.bui-pb__empty')?.textContent === 'No skills found', 'an agent without skills says so');
  sk.setValue('/');
  input(sta);
  ok(sk.el.querySelectorAll('.bui-pb__cmd').length === COMMANDS.length && !sk.el.querySelector('.bui-pb__heading'), 'a / after a $ shows the commands again');
  // An owner that answers the menu's opening at once, with a new list: that list is what the menu shows.
  const eager = bui.chatComposer({ placeholder: 'p', labels: composerLabels, agents: AGENTS, agentId: 'clark', commands: COMMANDS }, {
    onSlash: () => eager.setCommands([COMMANDS[0]]),
    onDollar: () => eager.setSkills({ items: [SKILLS[1]], heading: "Clark's skills" })
  });
  scratch.append(eager.el);
  const eta = eager.el.querySelector('.bui-pb__ta');
  eager.setValue('/');
  input(eta);
  ok([...eager.el.querySelectorAll('.bui-pb__cmdname')].map((n) => n.textContent).join() === '/reply', 'commands an owner sets as the menu opens are the ones it shows');
  eager.setValue('$');
  input(eta);
  ok([...eager.el.querySelectorAll('.bui-pb__cmdname')].map((n) => n.textContent).join() === '$invoice-check', 'skills an owner sets as the menu opens are the ones it shows');
  key(eta, 'Enter');
  ok(eager.getValue() === '$invoice-check ', 'and Enter picks from them');
  // Skill chips, as in t3code: a $name of the agent's skills becomes a chip once a space or punctuation follows it;
  // a name still being typed, one the agent doesn't have and an amount stay text. The text keeps $name.
  ok(bui.skillLabel({ name: 'codex:rescue' }) === 'Codex Rescue' && bui.skillLabel({ name: 'gh-fix-ci', displayName: 'Fix CI' }) === 'Fix CI', 'a chip names a skill by its words, or its display name');
  const chipSent = [];
  const cs = bui.chatComposer({ placeholder: 'p', labels: { ...composerLabels, skill: (n) => `Skill ${n}` }, agents: AGENTS, agentId: 'clark', commands: COMMANDS }, { onSend: (t) => chipSent.push(t) });
  scratch.append(cs.el);
  cs.setSkills({ items: SKILLS, heading: "Clark's skills" });
  const cta = cs.el.querySelector('.bui-pb__ta');
  const chips = () => [...cta.querySelectorAll('.bui-skill')].map((c) => c.dataset.skill).join();
  const select = (range) => {
    document.getSelection().removeAllRanges();
    document.getSelection().addRange(range);
  };
  ok(cta.classList.contains('is-empty') && cta.getAttribute('contenteditable') === 'plaintext-only', 'the input is a plain-text editable that shows its placeholder when empty');
  cta.focus();
  document.execCommand('insertText', false, 'Run $crm-update');
  ok(chips() === '' && cs.getValue() === 'Run $crm-update' && !cta.classList.contains('is-empty'), 'a name still being typed stays text');
  document.execCommand('insertText', false, ' now');
  const chip = cta.querySelector('.bui-skill');
  ok(chips() === 'crm-update' && cs.getValue() === 'Run $crm-update now', 'a space after it makes the name a chip, and the text keeps $name');
  ok(chip.textContent === 'Crm Update' && chip.getAttribute('aria-label') === 'Skill Crm Update' && chip.getAttribute('contenteditable') === 'false' && chip.querySelector('[data-icon="skill"]'), 'the chip has the box, the label and an accessible name');
  ok(chip.title === `$crm-update\n${SKILLS[2].description}`, 'its tooltip says the name and what the skill does');
  document.execCommand('insertText', false, '!');
  key(cta, 'z', { ctrlKey: true });
  ok(chips() === 'crm-update' && cs.getValue() === 'Run $crm-update now', 'Ctrl+Z undoes what was typed after the chip');
  key(cta, 'z', { ctrlKey: true });
  ok(chips() === '' && cs.getValue() === 'Run $crm-update', 'then the chip and the space that made it');
  key(cta, 'y', { ctrlKey: true });
  key(cta, 'z', { ctrlKey: true, shiftKey: true });
  ok(chips() === 'crm-update' && cs.getValue() === 'Run $crm-update now!', 'Ctrl+Y and Ctrl+Shift+Z redo');
  const after = document.createRange();
  after.setStartAfter(cta.querySelector('.bui-skill'));
  select(after);
  document.execCommand('delete');
  ok(chips() === '' && cs.getValue() === 'Run  now!', 'Backspace after a chip takes it whole');
  cs.setValue('a $invoice-check b');
  const all = document.createRange();
  all.selectNodeContents(cta);
  select(all);
  const clip = new DataTransfer();
  cta.dispatchEvent(new ClipboardEvent('copy', { clipboardData: clip, bubbles: true, cancelable: true }));
  ok(clip.getData('text/plain') === 'a $invoice-check b', 'a copied chip is its $name');
  const cutClip = new DataTransfer();
  select(all);
  cta.dispatchEvent(new ClipboardEvent('cut', { clipboardData: cutClip, bubbles: true, cancelable: true }));
  ok(cutClip.getData('text/plain') === 'a $invoice-check b' && cs.getValue() === '' && cta.classList.contains('is-empty'), 'a cut takes the chip along and empties the input');
  const pasted = new DataTransfer();
  pasted.setData('text/plain', 'Check with\r\n$invoice-check');
  cta.dispatchEvent(new ClipboardEvent('paste', { clipboardData: pasted, bubbles: true, cancelable: true }));
  ok(chips() === 'invoice-check' && cs.getValue() === 'Check with\n$invoice-check ', 'a pasted skill is a chip at once, with a space after it');
  cs.setValue('$nothing-here, $5 and $meeting-prep');
  ok(chips() === 'meeting-prep', 'a name the agent does not have, and an amount, stay text');
  cs.setSkills({ loading: true, heading: "Claude's skills" });
  ok(chips() === 'meeting-prep', 'chips wait while another list loads');
  cs.setSkills({ items: [SKILLS[1]], heading: "Claude's skills" });
  ok(chips() === '' && cs.getValue() === '$nothing-here, $5 and $meeting-prep', 'a chip for a skill the agent does not have is text again');
  cs.setSkills({ items: SKILLS, heading: "Clark's skills" });
  cs.setValue('');
  document.execCommand('insertText', false, 'Use $');
  cta.dispatchEvent(new Event('input', { bubbles: true }));
  key(cta, 'ArrowDown');
  key(cta, 'Enter');
  ok(chips() === 'invoice-check' && cs.getValue() === 'Use $invoice-check ', 'a skill picked from the menu goes in as its chip');
  document.execCommand('insertText', false, 'please');
  ok(cs.getValue() === 'Use $invoice-check please', 'typing goes on after the chip and its space');
  key(cta, 'Enter');
  ok(chipSent[0] === 'Use $invoice-check please' && cs.getValue() === '' && cta.classList.contains('is-empty'), 'Enter sends the text with $name and empties the input');
  const ub = bui.userBubble({ text: 'Run $crm-update, then $nope', skills: SKILLS, labels: { skill: (n) => `Skill ${n}` } });
  ok([...ub.querySelectorAll('.bui-skill')].map((c) => c.dataset.skill).join() === 'crm-update' && ub.textContent === 'Run Crm Update, then $nope', 'a sent message shows its skills as chips');
  ok(bui.userBubble({ text: 'Run $crm-update' }).querySelectorAll('.bui-skill').length === 0, 'without the list a message stays text');
  scratch.append(ub);
  const ubAll = document.createRange();
  ubAll.selectNodeContents(ub);
  select(ubAll);
  const ubClip = new DataTransfer();
  const onCopy = (e) => bui.copySkills(e);
  scratch.addEventListener('copy', onCopy);
  ub.dispatchEvent(new ClipboardEvent('copy', { clipboardData: ubClip, bubbles: true, cancelable: true }));
  scratch.removeEventListener('copy', onCopy);
  ok(ubClip.getData('text/plain') === 'Run $crm-update, then $nope', 'a copied message gives its chips as $name');

  // A chip that no longer reads as its skill is text again: here a letter typed right after it.
  cs.setValue('Use $crm-update now');
  cta.focus();
  const behind = document.createRange();
  behind.setStartAfter(cta.querySelector('.bui-skill'));
  select(behind);
  document.execCommand('insertText', false, 'x');
  ok(chips() === '' && cs.getValue() === 'Use $crm-updatex now', 'a chip with a letter typed right after it is text again');
  // Undo after another agent's list came in shows that list's chips, not the old ones.
  cs.setValue('');
  document.execCommand('insertText', false, 'Use $crm-update ');
  document.execCommand('insertText', false, 'now');
  cs.setSkills({ items: [SKILLS[1]], heading: "Claude's skills" });
  key(cta, 'z', { ctrlKey: true });
  ok(chips() === '' && cs.getValue() === 'Use $crm-update ', 'an undone step has no chip for a skill the agent does not have');
  // A list that comes in keeps the selection the user has.
  cs.setSkills({ loading: true, heading: "Clark's skills" });
  cs.setValue('Run $crm-update now');
  const whole = document.createRange();
  whole.selectNodeContents(cta);
  select(whole);
  cs.setSkills({ items: SKILLS, heading: "Clark's skills" });
  const kept = new DataTransfer();
  cta.dispatchEvent(new ClipboardEvent('copy', { clipboardData: kept, bubbles: true, cancelable: true }));
  ok(chips() === 'crm-update' && kept.getData('text/plain') === 'Run $crm-update now', 'a list that comes in keeps the selection');
  // Lines the browser adds as elements become text, empty lines included.
  cta.innerHTML = 'a<div><br></div><div>b $crm-update c</div>';
  cta.dispatchEvent(new Event('input', { bubbles: true }));
  ok(cs.getValue() === 'a\n\nb $crm-update c' && chips() === 'crm-update' && ![...cta.children].some((c) => !c.dataset.skill), 'browser lines become text, an empty line included');

  let edits = 0;
  const edit = bui.suggestionEdit({ label: 'Edit shortcuts', onClick: () => edits++ });
  edit.click();
  ok(edit.getAttribute('aria-label') === 'Edit shortcuts' && edits === 1, 'the edit chip has a name and calls back');

  scratch.remove();
  return fails;
}

// Run with reduced motion asked for (the script emulates it): what moves must stop. A pending attachment's name
// stays still in the supporting text colour. Its shimmer is switched off, so the check reads the style at once: a
// shortened animation is in getAnimations() until a frame has run it, and the script's off-screen window may not
// draw one in time.
async function checkReducedMotion() {
  if (!matchMedia('(prefers-reduced-motion: reduce)').matches) return ['reduced motion is not being emulated'];
  const fails = [];
  const scratch = document.createElement('div');
  scratch.className = 'bui';
  scratch.style.cssText = 'position:fixed;left:0;top:0;width:380px';
  document.body.append(scratch);
  const pend = bui.attachmentChip({ label: 'b.png', sub: '2 KB', pending: true, busy: 'Adding b.png' });
  scratch.append(pend);
  const name = pend.querySelector('.bui-entity__name');
  const style = getComputedStyle(name);
  if (style.animationName !== 'none' || name.getAnimations().length) fails.push('a pending attachment name still shimmers with reduced motion');
  if (style.backgroundImage !== 'none' || /^rgba\(.*, 0\)$|^transparent$/.test(style.color)) fails.push('a pending attachment name is drawn in one solid colour with reduced motion');
  if (pend.getAttribute('aria-label') !== 'Adding b.png') fails.push('a pending attachment keeps its name with reduced motion');
  scratch.remove();
  return fails;
}

// Hooks for the screenshot script.
window.gallery = {
  restart(menu) {
    run();
    if (menu === 'slash') {
      c4.setValue('/re');
      c4.focus();
      c4.el.querySelector('.bui-pb__ta').dispatchEvent(new Event('input'));
    }
  },
  openAgents() {
    c1.el.querySelector('.bui-pb__modelbtn').click();
  },
  check,
  checkReducedMotion,
  paused
};
window.bui = bui;
run();
