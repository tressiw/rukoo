import { t, getLocale } from '../i18n.js';
// The chat panel: a conversation with Clark (Hermes), Claude Code or Codex about the open email, or in meeting mode
// about the Rukoo Voice meeting on screen. Main runs the agents and keeps the conversations; this module draws them,
// follows the email or meeting on screen and answers the agents' requests to write into the composer (the UI bridge).
import { icons } from '../icons.js';
import { api, $, toast, listTime, person, hue, fileSize } from '../ui.js';
import * as B from './bui.js';
import { shortcutList, shortcutByKey, actionsFor, commandsFor, skillCommands, MEETING_ACTIONS, meetingActionByKey } from './actions.js';
import { replyEnvelope } from '../composer.js';
import { expired } from './expired.js';

const AGENTS = ['clark', 'claude', 'codex', 'openclaw'];
// Product names, not translated.
const PRODUCTS = { clark: 'Hermes Agent', claude: 'Claude Code', codex: 'Codex', openclaw: 'OpenClaw' };
const STATES = ['starting', 'ready', 'offline', 'unconfigured', 'disabled', 'missing', 'unauthorized', 'unknown'];
// Until these are fixed in Settings the agent cannot take a message.
const BLOCKED = new Set(['unconfigured', 'disabled', 'missing', 'unauthorized']);
const CODES = new Set(['offline', 'unauthorized', 'not-configured', 'not-installed', 'disabled', 'spawn-failed', 'busy', 'window', 'timeout', 'protocol', 'rate-limited', 'stopped', 'unknown']);
// Rukoo's own tools: their kind picks the chip icon and their label is ours to translate.
const RUKOO_TOOLS = {
  get_context: 'mail',
  read_message: 'mail',
  search_mail: 'search',
  write_draft: 'draft',
  get_draft: 'draft',
  show_plan: 'plan',
  show_sources: 'sources',
  propose_action: 'approval',
  mail_action: 'approval',
  read_attachment: 'file',
  read_chat_file: 'file',
  read_skill: 'file',
  list_events: 'calendar',
  find_free_time: 'calendar',
  create_event: 'calendar',
  get_booking_link: 'link',
  read_meeting_transcript: 'meeting',
  search_meetings: 'search',
  read_meeting_notes: 'meeting',
  get_meeting_frame: 'meeting',
  get_meeting_clip: 'meeting',
  get_meeting_media_link: 'meeting'
};
const CHOICES = new Set(['allow', 'deny', 'once', 'session', 'always', 'approve', 'decline', 'accept', 'acceptForSession', 'cancel']);
// Main and the adapters label approval buttons in English. Those standard labels are matched back to
// our keys (through the English catalog) so they can be translated; a label the agent wrote itself,
// such as propose_action's "Add tasks", stays as it is.
let choiceKeys = null;
function choiceKey(label) {
  if (!choiceKeys) {
    const en = (globalThis.RukooLocales && globalThis.RukooLocales.en) || {};
    choiceKeys = new Map();
    for (const id of CHOICES) {
      const entry = en[`agent.choices.${id}`];
      if (entry && typeof entry.message === 'string') choiceKeys.set(entry.message.toLowerCase(), `agent.choices.${id}`);
    }
  }
  return choiceKeys.get(String(label || '').trim().toLowerCase()) || null;
}
// Runtime approvals name the kind of tool, so the card can say in the interface language what the agent wants.
const APPROVAL_TITLES = new Set(['command', 'readFile', 'changeFiles', 'web', 'permissions']);
// Field keys the adapters send; 'input' is anything else and keeps the adapter's own label.
const APPROVAL_FIELDS = new Set(['command', 'folder', 'file', 'path', 'url', 'query', 'pattern', 'content', 'before', 'after', 'diff', 'reason']);
const DRAFT_MODES = { reply: 'reply', reply_all: 'replyAll', replyAll: 'replyAll', forward: 'forward', new: 'new' };
const sameMessageId = (a, b) => Boolean(a && b) && String(a).replace(/[<>\s]/g, '').toLowerCase() === String(b).replace(/[<>\s]/g, '').toLowerCase();
const hasKey = (key) => Boolean(globalThis.RukooLocales && globalThis.RukooLocales.en && globalThis.RukooLocales.en[key]);
// Mail results main reports with counts, so they can be said in the interface language.
const MAIL_RESULTS = new Set(['archive', 'trash', 'move', 'mark_read', 'mark_unread', 'star', 'unstar', 'undo']);

const elOf = (x) => (x instanceof Node ? x : x && x.el);
// Main marks text from email with <unsafe_content> tags for the agents. A proposal or a plan keeps the tags of
// email text the agent quoted in it, so Rukoo can repeat it to the agent still marked; the user reads the text
// without them. The same pattern as untag() in main's agents/context.js.
const UNSAFE_TAG = /<\/?unsafe_content(?:\s(?:"[^"]*"|[^">])*)?>/gi;
const untag = (v) => (typeof v === 'string' ? v.replace(UNSAFE_TAG, '') : v);
const rukooTool = (name) => String(name || '').replace(/^(mcp__rukoo__|mcp_rukoo_|rukoo[._:])/i, '');
// These tools put their own card in the transcript (the draft, the plan, the sources, the approval),
// so once they succeed a chip saying the same thing is noise. Running and failed calls keep theirs.
const CARD_TOOLS = new Set(['write_draft', 'show_plan', 'show_sources', 'propose_action', 'mail_action', 'create_event']);
const cardToolDone = (item) => {
  const tool = rukooTool(item.name);
  return item.status === 'done' && CARD_TOOLS.has(tool) && (item.rukoo || tool !== item.name);
};

function toolKind(item) {
  const tool = rukooTool(item.name);
  if (RUKOO_TOOLS[tool]) return RUKOO_TOOLS[tool];
  const n = String(item.name || '').toLowerCase();
  if (/bash|shell|powershell|terminal|command|exec/.test(n) || /^ran a command/i.test(item.label || '')) return 'command';
  if (/web|fetch|browse|url/.test(n)) return 'web';
  if (/read|write|edit|file|glob|grep|patch/.test(n)) return 'file';
  if (/mail/.test(n)) return 'mail';
  if (/search/.test(n)) return 'search';
  return 'tool';
}

// The agents' own common tools, across Claude Code, Codex and Hermes, in the interface language.
// Other MCP servers read as "server: tool", anything else as its name with spaces.
const GENERIC_TOOLS = [
  [/^(bash|terminal|shell|powershell|command|commandexecution|exec(_command)?|run_command|local_shell)$/i, 'command'],
  [/^(execute_code|code_execution)$/i, 'code'],
  [/^(websearch|web_search|websearchtool|search_web|x_search)$/i, 'webSearch'],
  [/^(webfetch|web_fetch|web_extract|fetch_url)$/i, 'webPage'],
  [/^browser/i, 'browser'],
  [/^(read|read_file|view|cat|notebookread)$/i, 'readFile'],
  [/^(write|edit|multiedit|filechange|apply_patch|write_file|patch|notebookedit)$/i, 'changeFiles'],
  [/^(task|agent|delegate_task|subagent.*|collabagenttoolcall)$/i, 'subagent'],
  [/^(glob|grep|ls|search_files)$/i, 'findFiles'],
  [/^(todowrite|todo)$/i, 'todo'],
  [/^(toolsearch|tool_describe|tools_list)$/i, 'lookupTools'],
  [/^(memory|honcho_.+|mem0_.+)$/i, 'memory'],
  [/^(skill|skill_view|skills_list|skill_manage)$/i, 'skill'],
  [/^session_search$/i, 'pastChats'],
  [/^cronjob$/i, 'schedule'],
  [/^send_message$/i, 'sendMessage'],
  [/^(image_generate|imagegeneration)$/i, 'image'],
  [/^(vision_analyze|imageview)$/i, 'vision']
];

// "mcp__strap__strap_search" reads as "strap: search", "mcp__todoist__add_task" as "todoist: add task".
function mcpLabel(name) {
  const m = /^mcp__([^_].*?)__(.+)$/.exec(name) || /^mcp_([a-z0-9-]+)_(.+)$/i.exec(name);
  if (!m) return null;
  const server = m[1];
  let tool = m[2];
  if (tool.toLowerCase().startsWith(`${server.toLowerCase()}_`)) tool = tool.slice(server.length + 1);
  return `${server.replace(/_/g, ' ')}: ${tool.replace(/_/g, ' ')}`;
}

// While a tool runs its chip says what it is doing ("Searching your mail…"), afterwards what it did.
function toolProps(item) {
  const tool = rukooTool(item.name);
  const name = String(item.name || '');
  const tense = item.status === 'running' ? 'running.' : '';
  let label;
  if (RUKOO_TOOLS[tool] && (item.rukoo || tool !== item.name)) label = t(`agent.tools.${tense}${tool}`);
  else {
    const generic = GENERIC_TOOLS.find(([re]) => re.test(name));
    if (generic) label = t(`agent.tools.${tense}generic.${generic[1]}`);
    else label = mcpLabel(name) || (item.label && item.label !== name ? item.label : name.replace(/_/g, ' ').trim()) || t(`agent.tools.${tense}unknown`);
  }
  return { ...item, label, kind: toolKind(item) };
}

// "busy", "offline: connect ECONNREFUSED …" or an object with a code: a sentence for people plus the technical detail.
function describeError(err, name) {
  const source = err && err.code ? err : err && err.error && err.error.code ? err.error : null;
  let code = source ? source.code : null;
  let detail = source ? String(source.detail || '') : String((err && err.message) || err || '');
  const m = !code && detail.match(/^([a-z-]+)(?::\s*([\s\S]*))?$/);
  if (m && CODES.has(m[1])) {
    code = m[1];
    detail = m[2] || '';
  }
  if (!CODES.has(code)) return { code: null, text: detail || t('agent.errors.unknown', { name }), detail: '' };
  return { code, text: t(`agent.errors.${code}`, { name }), detail };
}

export function mountAgentPanel(ctx) {
  const { S } = ctx;
  const pane = $('.agentpane');
  const P = {
    config: null,
    status: null,
    unavailable: false,
    // The agent a new conversation goes to.
    agentId: 'clark',
    conv: null,
    // The email the panel last followed, and whether it had fully loaded.
    emailId: null,
    emailKey: undefined,
    // The user left the email out of the chat that has not started yet: it starts without it, unless the
    // add chip puts it back. Another email or New chat ends this.
    detached: false,
    // For an email without a chat: the most recent chat about an earlier email in its thread, which the empty
    // state offers to continue ({id, agent, title, updatedAt, for: the email's id}).
    related: null,
    // An email opened from the panel itself (a source, a draft): the chat stays as it is.
    keepFor: null,
    // A meeting chat picked from the chat list opens its meeting; that chat stays, not the newest one about it.
    keepMeeting: null,
    // Rukoo's skills and the user's, for the slash commands: [{name, description, source}].
    skills: [],
    // The quick actions as the user arranged them in Settings (actions.js shortcutList), for every agent.
    shortcuts: shortcutList(null),
    // agent → its own skills for the $ menu (agentOwnSkills: {skills, error}), and the agents whose list is loading.
    ownSkills: {},
    loadingSkills: new Set(),
    // What goes along with the message being written: files main keeps until it is sent ({id, name, size, kind}),
    // and emails from the list ({id, subject, from}). Like the text in the input, they stay when another chat opens.
    attach: { files: [], emails: [] },
    // Dropped or pasted files main is still being handed ({name, size}): shown as pending chips until it has them.
    adding: [],
    // The model and effort picked for a chat that has not started yet (null: the agent's default). A started chat
    // keeps its own on the conversation.
    draft: { model: null, effort: null },
    // agent → the models and efforts main offers (agentModels), and the agents whose list is loading.
    models: {},
    loadingModels: new Set(),
    // Main's list of chats, newest message first (agentList), for the bell and its menu.
    sessions: [],
    // The chat main last heard is on screen (agentWatch): undefined until the panel has told it.
    watching: undefined
  };

  pane.innerHTML = `
    <header class="ap-head">
      <span class="ap-mono" aria-hidden="true"></span>
      <div class="ap-who">
        <div class="ap-line"><span class="ap-agent"></span><span class="ap-dot" data-state="starting"></span></div>
        <div class="ap-title"></div>
      </div>
      <button class="icon-btn sm ap-bell" data-ap="history" aria-haspopup="menu">${icons.bell}<span class="ap-count" aria-hidden="true" hidden></span></button>
      <button class="icon-btn sm" data-ap="new">${icons.plus}</button>
      <button class="icon-btn sm" data-ap="close">${icons.close}</button>
    </header>
    <div class="ap-body">
      <div class="ap-scroll" tabindex="-1"><div class="ap-transcript" role="log"></div></div>
      <button class="ap-jump" data-ap="jump" hidden>${icons.down}</button>
      <div class="ap-drop" aria-hidden="true"><span class="ap-drop-label"></span></div>
    </div>
    <div class="ap-foot"></div>`;
  const head = {
    mono: $('.ap-mono', pane),
    agent: $('.ap-agent', pane),
    dot: $('.ap-dot', pane),
    title: $('.ap-title', pane)
  };
  const scroller = $('.ap-scroll', pane);
  const transcript = $('.ap-transcript', pane);
  // A skill chip in a sent message copies as its $name.
  transcript.addEventListener('copy', (e) => B.copySkills(e));
  const jump = $('[data-ap="jump"]', pane);
  const foot = $('.ap-foot', pane);

  // ---------- agents ----------

  const currentAgent = () => (P.conv ? P.conv.agent : P.agentId);
  // Custom agents (Settings > Agents > Add an agent) come after the built-in ones. A chat with one the user removed
  // keeps its name until the panel reloads.
  const customOf = (id) => (P.config && Array.isArray(P.config.custom) && P.config.custom.find((x) => x.id === id)) || null;
  const agentIds = () => [...AGENTS, ...((P.config && P.config.custom) || []).map((x) => x.id)];
  const productOf = (id) => {
    const x = customOf(id);
    return x ? t(`settings.agents.preset.${x.preset || 'runs'}`) : PRODUCTS[id];
  };
  const agentName = (id) => {
    if (/^cx_[a-z0-9]{10}$/.test(String(id || ''))) return (customOf(id) || {}).name || t('settings.agents.removedAgent');
    return id === 'clark' ? (P.config && P.config.clark && P.config.clark.name) || 'Hermes' : id === 'claude' ? 'Claude' : id === 'codex' ? 'Codex' : id === 'openclaw' ? 'OpenClaw' : String(id || '');
  };
  const stateOf = (id) => {
    const s = P.unavailable ? 'unknown' : P.status && P.status[id] ? P.status[id].state : 'starting';
    return STATES.includes(s) ? s : 'unknown';
  };
  const dotOf = (state) => (state === 'ready' ? 'ready' : state === 'offline' || state === 'unauthorized' ? 'offline' : state === 'starting' ? 'starting' : 'unconfigured');
  // Whether the agent takes a message during its turn (main's status says, per agent). Until the status is in, it is
  // offered; main queues the message when the agent can't take it after all.
  const canSteer = (id) => !(P.status && P.status[id] && P.status[id].steer === false);
  const statusText = (id) => t(`agent.status.${stateOf(id)}`);
  const agentRows = () =>
    agentIds().map((id) => ({ id, name: agentName(id), label: agentName(id), tag: productOf(id), status: dotOf(stateOf(id)), checked: id === currentAgent() }));

  // The config is local and answers at once; status can wait on the network (an offline Hermes server takes
  // seconds). So the default agent and the panel come first, and the status fills in when it arrives.
  async function loadAgents() {
    try {
      const config = await api('agentConfig');
      const before = P.config ? P.config.defaultAgent : null;
      P.config = config;
      P.shortcuts = shortcutList(config.shortcuts);
      P.unavailable = false;
      // A new default in Settings applies to the next new chat; an open chat keeps its own agent.
      if (agentIds().includes(config.defaultAgent) && config.defaultAgent !== before) {
        // What was picked for the chat that has not started belongs to the agent before.
        if (!P.conv && P.agentId !== config.defaultAgent) P.draft = { model: null, effort: null, permission: P.draft.permission || null };
        P.agentId = config.defaultAgent;
      }
    } catch (err) {
      // No agent support in main (yet): show the panel, but say so instead of failing quietly.
      if (!P.config) P.unavailable = true;
    }
    redrawAgents();
    if (P.unavailable) return;
    // A new model in Settings: main dropped its list, and this gets the new one.
    if (ctx.agentOpen()) loadModels();
    try {
      P.status = await api('agentStatus');
    } catch (_) {
      // The status lines keep their last known state.
    }
    redrawAgents();
  }

  function redrawAgents() {
    renderHead();
    refreshComposer();
    if (!P.conv) renderEmpty();
  }

  // ---------- header ----------

  function renderHead() {
    const id = currentAgent();
    const name = agentName(id);
    head.mono.replaceChildren(elOf(B.monogram({ label: [...name][0] || '?', size: 24, agent: true })));
    head.agent.textContent = name;
    head.dot.dataset.state = dotOf(stateOf(id));
    head.dot.title = statusText(id);
    head.title.textContent = P.conv ? P.conv.title || t('agent.panel.newChat') : P.unavailable ? t('agent.status.unavailable') : statusText(id);
    head.title.title = head.title.textContent;
  }

  function localizeChrome() {
    for (const [action, key] of [['new', 'agent.panel.new'], ['close', 'agent.panel.close'], ['jump', 'agent.panel.jump']]) {
      const b = $(`[data-ap="${action}"]`, pane);
      b.title = t(key);
      b.setAttribute('aria-label', t(key));
    }
    // The bell's label says how many chats have news.
    renderNews();
    transcript.setAttribute('aria-label', t('agent.panel.transcript'));
    $('.ap-drop-label', pane).textContent = t('agent.panel.dropHere');
  }

  // ---------- chat input ----------

  let chat = null;
  const contextEmail = () => (P.conv ? P.conv.message : P.detached ? null : currentEmail());
  // The meeting a chat is about: the chat's own, or before it starts the one on screen in meeting mode.
  const contextMeeting = () => (P.conv ? P.conv.meeting || null : currentMeeting());
  // A meeting chat's own quick actions come first as slash commands too.
  const meetingCommands = () => (contextMeeting() ? MEETING_ACTIONS.map((a) => ({ name: a.key, label: a.label, description: a.description, icon: a.icon })) : []);
  // The slash commands go by the email itself when it is on screen: it knows whether it can unsubscribe,
  // also for chats saved before their reference carried that.
  const slashEmail = () => {
    const m = contextEmail();
    const open = currentEmail();
    return m && open && open.id === m.id ? open : m;
  };
  // The shortcuts for the email, then the skills.
  const commands = () => [...meetingCommands(), ...commandsFor(slashEmail(), P.shortcuts), ...skillCommands(P.skills, P.shortcuts)];
  // Main reads the skills folders on every call, so a skill the user just added shows up the next time they type /.
  let skillsLoading = null;
  let skillsAgain = false;
  function loadSkills() {
    // A load already under way may have read the folders before the latest change: read them once more after it.
    if (skillsLoading) {
      skillsAgain = true;
      return skillsLoading;
    }
    skillsLoading = api('agentSkills')
      .then((list) => {
        P.skills = Array.isArray(list) ? list : [];
        if (chat) chat.setCommands(commands());
      })
      .catch(() => {})
      .finally(() => {
        skillsLoading = null;
        if (skillsAgain) {
          skillsAgain = false;
          loadSkills();
        }
      });
    return skillsLoading;
  }
  const placeholder = () => {
    const name = agentName(currentAgent());
    if (contextMeeting()) return t('agent.panel.placeholderMeeting', { name });
    return contextEmail() ? t('agent.panel.placeholderEmail', { name }) : t('agent.panel.placeholder', { name });
  };
  // In a chat the user continued from a newer email in its thread (main's continueFrom): that email, while it is
  // on screen, and whether it still has to go to the agent.
  const newerOpen = () => {
    const open = currentEmail();
    const conv = P.conv;
    if (!conv || !conv.message || !open || open.id === conv.message.id) return null;
    const entry = (conv.continued || []).find((e) => e.id === open.id || sameMessageId(e.messageId, open.messageId));
    return entry ? { m: open, pending: Boolean(entry.pending) } : null;
  };
  // The email the chat is about. Until the chat starts, its x leaves the email out and a dashed chip in its
  // place puts it back. A chat that has started keeps its email: New chat starts one without. A chat continued
  // from a newer email shows that one next to it while it is open.
  const contextState = () => {
    const meeting = contextMeeting();
    if (meeting) return { meeting };
    const m = contextEmail();
    if (m) return { m, fixed: Boolean(P.conv), newer: newerOpen() };
    const open = !P.conv && P.detached ? currentEmail() : null;
    return open ? { m: open, add: true } : null;
  };
  const contextKey = (s) =>
    s && s.meeting ? ['meeting', s.meeting.id, s.meeting.title || '', s.meeting.deleted === true].join('\n') : s ? [s.add ? 'add' : s.fixed ? 'fixed' : 'open', s.m.id, s.m.subject, person(s.m.from), s.newer ? `${s.newer.m.id}\n${s.newer.m.subject}\n${s.newer.pending}` : ''].join('\n') : '';
  const contextChip = (s) => {
    if (!s) return null;
    // A meeting stays with its chat: no x, as for an email once the chat started.
    if (s.meeting) {
      const badge = document.createElement('span');
      badge.className = 'bui-entity__badge';
      badge.setAttribute('aria-hidden', 'true');
      badge.append(B.icon('mic', 10, 2.2));
      return elOf(B.entityChip({ label: s.meeting.title || t('meeting.untitledNoTime'), sub: t(s.meeting.deleted ? 'agent.panel.meetingDeleted' : 'agent.panel.meetingChip'), monogram: badge }));
    }
    const m = s.m;
    const subject = m.subject || t('mailbox.message.noSubject');
    if (s.add) return elOf(B.addChip({ label: t('agent.panel.addContext'), title: subject, onClick: () => attach() }));
    const from = person(m.from);
    const own = elOf(
      B.entityChip({
        label: subject,
        sub: from,
        monogram: { label: from || '?', hue: hue((m.from && m.from.address) || from) },
        onRemove: s.fixed ? null : () => detach(),
        removeLabel: t('agent.panel.removeContext')
      })
    );
    if (!s.newer) return own;
    const badge = document.createElement('span');
    badge.className = 'bui-entity__badge';
    badge.setAttribute('aria-hidden', 'true');
    badge.append(B.icon('mail', 10, 2.2));
    const newer = elOf(B.entityChip({ label: s.newer.m.subject || t('mailbox.message.noSubject'), sub: t('agent.panel.newerMessage'), monogram: badge }));
    const name = agentName(P.conv.agent);
    const note = newer.querySelector('.bui-entity__sub');
    if (note) note.title = s.newer.pending ? t('agent.panel.newerPending', { name }) : t('agent.panel.newerSent', { name });
    const both = document.createDocumentFragment();
    both.append(own, newer);
    return both;
  };
  // Drawn again only when it changes, so its button keeps the keyboard focus through a status update.
  let shownContext = null;
  function showContext() {
    const s = contextState();
    const key = contextKey(s);
    if (key === shownContext) return;
    shownContext = key;
    chat.setContext(contextChip(s));
  }

  function buildComposer() {
    const value = chat ? chat.getValue() : '';
    if (chat) chat.el.remove();
    const context = contextState();
    shownContext = contextKey(context);
    chat = B.chatComposer(
      {
        placeholder: placeholder(),
        labels: {
          send: t('agent.panel.send'),
          stop: t('agent.panel.stop'),
          agent: t('agent.panel.pickAgent'),
          removeContext: t('agent.panel.removeContext'),
          noMatches: t('agent.panel.noMatches'),
          skillsLoading: t('agent.panel.skillsLoading'),
          noSkills: t('agent.panel.noSkills'),
          skill: (name) => t('agent.panel.skillChip', { name }),
          attach: t('agent.panel.attach'),
          steer: t('agent.panel.steer'),
          queue: t('agent.panel.queue'),
          steerHint: (name) => t('agent.panel.steerHint', { name }),
          queueHint: (name) => t('agent.panel.queueHint', { name }),
          cantSteer: (name) => t('agent.panel.cantSteer', { name }),
          sendOptions: t('agent.panel.sendOptions')
        },
        agents: agentRows(),
        agentId: currentAgent(),
        commands: commands(),
        context: contextChip(context)
      },
      {
        onSend: (text, mode) => submit(text, mode),
        onStop: () => P.conv && api('agentStop', P.conv.id).catch((err) => showError(err)),
        onAgent: (id) => pickAgent(id),
        onPick: (kind, id) => pickChoice(kind, id),
        onCommand: (name) => runAction(name),
        onSlash: () => loadSkills(),
        onDollar: () => loadOwnSkills(currentAgent(), true),
        onRemoveContext: () => detach(),
        onAttach: () => pickFiles(),
        onFiles: (list) => addFiles(list)
      }
    );
    foot.append(chat.el);
    if (value) chat.setValue(value);
    showAttachments();
    showOwnSkills();
  }

  // ---------- attachments ----------

  // The same caps as main's; main checks again.
  const FILE_MAX = 10 * 1024 * 1024;
  const MAX_FILES = 10;
  const MAX_EMAILS = 10;
  const FILE_ICONS = { pdf: 'file-text', text: 'file-text', image: 'image', file: 'file' };
  const fileChip = (f, onRemove) =>
    elOf(B.attachmentChip({ label: f.name, sub: fileSize(f.size), icon: FILE_ICONS[f.kind] || 'file', onRemove, removeLabel: t('agent.panel.removeAttachment', { name: f.name }) }));
  // A mail badge rather than the sender's monogram, so an added email reads as an attachment next to the chat's own.
  const emailChip = (m, onRemove) => {
    const subject = m.subject || t('mailbox.message.noSubject');
    return elOf(B.attachmentChip({ label: subject, sub: person(m.from), icon: 'mail', onRemove, removeLabel: t('agent.panel.removeAttachment', { name: subject }) }));
  };

  const pendingChip = (f) => elOf(B.attachmentChip({ label: f.name, sub: fileSize(f.size), icon: 'file', pending: true, busy: t('agent.files.adding', { name: f.name }) }));

  function showAttachments() {
    if (!chat) return;
    const { files, emails } = P.attach;
    chat.setAttachments([...files.map((f) => fileChip(f, () => removeFile(f.id))), ...P.adding.map(pendingChip), ...emails.map((m) => emailChip(m, () => removeEmail(m.id)))]);
    refreshHolding();
  }

  // While a file is still on its way (a pending chip, or the paperclip's files being read) nothing is sent or
  // started: the message would go without it, or it would land on the next one. Send is off, Enter leaves the text
  // as it is, and quick actions and skills say why. Once the files are there, the user sends.
  let picking = 0;
  const holding = () => P.adding.length > 0 || picking > 0;
  function refreshHolding() {
    if (chat) chat.setHolding(holding(), t('agent.panel.waitFiles'));
  }

  function removeFile(id) {
    P.attach.files = P.attach.files.filter((f) => f.id !== id);
    api('agentDropFile', id).catch(() => {});
    showAttachments();
  }

  function removeEmail(id) {
    P.attach.emails = P.attach.emails.filter((m) => m.id !== id);
    showAttachments();
  }

  // Short notices, one line each: what could not go along, and why.
  function refusals(list) {
    const lines = list.filter(Boolean);
    if (lines.length) toast(lines.join(' '), 6000);
  }

  const REFUSALS = { blocked: 'agent.files.blocked', 'too-big': 'agent.files.tooBig', full: 'agent.files.full', folder: 'agent.files.folder' };
  const refusal = (r) => t(REFUSALS[r.error] || 'agent.files.unreadable', { name: r.name });

  // What main staged ({id, name, size, kind}) or refused ({name, error}); 'too-many': past what the message takes.
  // Past MAX_FILES, main lets go of the rest.
  function takeFiles(results) {
    const notes = [];
    let full = false;
    for (const r of Array.isArray(results) ? results : []) {
      if (r.error === 'too-many') full = true;
      else if (r.error) notes.push(refusal(r));
      else if (P.attach.files.length >= MAX_FILES) {
        full = true;
        api('agentDropFile', r.id).catch(() => {});
      } else P.attach.files.push({ id: r.id, name: r.name, size: r.size, kind: r.kind });
    }
    if (full) notes.push(t('agent.files.tooMany', { count: MAX_FILES }));
    showAttachments();
    refusals(notes);
  }

  // The paperclip: main's own file dialog. Main stages no more than the message still takes.
  async function pickFiles() {
    picking++;
    refreshHolding();
    try {
      takeFiles(await api('agentPickFiles', Math.max(0, MAX_FILES - P.attach.files.length - P.adding.length)));
    } catch (err) {
      showError(err);
    } finally {
      picking--;
      refreshHolding();
    }
    chat.focus();
  }

  // Dropped or pasted files: [{file, folder}]. The renderer has no path it may hand on, so main gets the bytes. What
  // goes along shows at once as a pending chip, before any of it is read, and holds the message until it is there.
  async function addFiles(list) {
    const results = [];
    const going = [];
    let room = MAX_FILES - P.attach.files.length - P.adding.length;
    for (const { file, folder } of list) {
      if (folder) results.push({ name: file.name, error: 'folder' });
      else if (file.size > FILE_MAX) results.push({ name: file.name, error: 'too-big' });
      else if (room <= 0) results.push({ name: file.name, error: 'too-many' });
      else {
        room--;
        const pending = { name: file.name, size: file.size };
        going.push({ file, pending, at: results.push(null) - 1 });
      }
    }
    P.adding.push(...going.map((g) => g.pending));
    showAttachments();
    for (const g of going) {
      try {
        results[g.at] = await api('agentAddFile', { name: g.file.name, data: new Uint8Array(await g.file.arrayBuffer()) });
      } catch (_) {
        results[g.at] = { name: g.file.name, error: 'unreadable' };
      }
    }
    // The pending chips give way to the files main has, all at once, so none blinks out in between.
    P.adding = P.adding.filter((p) => !going.some((g) => g.pending === p));
    takeFiles(results);
  }

  // An email the chat has already: its own, or a newer one in its thread it was continued from (main's continueFrom),
  // which the next message carries by itself while the agent has not had it. Added again, it would go twice. One
  // email can be in several folders under several ids, so its Message-ID counts as well.
  const sameEmail = (a, b) => a.id === b.id || sameMessageId(a.messageId, b.messageId);
  function inChat(m) {
    const own = contextEmail();
    if (own && sameEmail(own, m)) return true;
    return Boolean(P.conv && (P.conv.continued || []).some((e) => sameEmail(e, m)));
  }

  // The emails dragged from the list or picked from a menu go with the message, up to MAX_EMAILS; how many were
  // new, and how many were already in the chat or the message.
  function addEmails(ids) {
    let full = false;
    let added = 0;
    let had = 0;
    for (const id of Array.isArray(ids) ? ids : []) {
      const m = ctx.messageById(id);
      if (!m) continue;
      if (inChat(m) || P.attach.emails.some((x) => sameEmail(x, m))) {
        had++;
        continue;
      }
      if (P.attach.emails.length >= MAX_EMAILS) {
        full = true;
        continue;
      }
      P.attach.emails.push({ id: m.id, messageId: m.messageId || null, subject: m.subject || '', from: m.from ? { name: m.from.name || '', address: m.from.address || '' } : null });
      added++;
    }
    showAttachments();
    if (full) toast(t('agent.files.tooManyEmails', { count: MAX_EMAILS }), 6000);
    return { added, had };
  }

  const hasAttachments = () => P.attach.files.length > 0 || P.attach.emails.length > 0;

  // Why the input is off, with the way to fix it: "Hermes isn't set up yet. Set up Hermes".
  function setupNote(id, state) {
    const name = agentName(id);
    const note = document.createElement('span');
    note.className = 'ap-setup';
    const text = document.createElement('span');
    text.textContent = P.unavailable ? t('agent.status.unavailable') : t(`agent.setup.${state}`, { name });
    note.append(text);
    if (!P.unavailable) {
      const link = document.createElement('button');
      link.type = 'button';
      link.className = 'link-btn';
      link.dataset.ap = 'setup';
      link.textContent = t('agent.panel.setUp', { name });
      note.append(link);
    }
    return note;
  }

  // Agent, email, commands and availability follow what is on screen.
  function refreshComposer() {
    if (!chat) return;
    const id = currentAgent();
    chat.setAgents(agentRows(), id);
    showContext();
    chat.setCommands(commands());
    showOwnSkills();
    chat.setRunning(Boolean(P.conv && P.conv.status === 'running'));
    chat.setSteer(canSteer(id));
    chat.setPlaceholder(placeholder());
    const state = stateOf(id);
    const blocked = P.unavailable || BLOCKED.has(state);
    chat.setDisabled(blocked, blocked ? setupNote(id, state) : null);
    refreshPickers();
    // Whether the agent can steer decides what a waiting message offers.
    showQueue();
  }

  // ---------- model and effort ----------

  // The chat on screen's own model, effort and permission level, or what was picked for the chat that has not started
  // yet.
  const choiceOf = () => {
    const c = P.conv || P.draft;
    return { model: c.model || null, effort: c.effort || null, permission: c.permission || null };
  };
  const effortName = (e) => (hasKey(`agent.effort.${e}`) ? t(`agent.effort.${e}`) : String(e));
  // Where the effort dial's needle stands for each level, so a narrow panel still shows it without the name.
  const METER = { minimal: 0, low: 0.15, medium: 0.35, high: 0.55, xhigh: 0.75, max: 0.9, ultra: 1 };
  const modelEntry = (list, id) => (list && id ? list.models.find((m) => m.id === id) || null : null);
  // Hermes names a model of another provider "provider::model"; the menus show the model.
  const modelName = (list, id) => {
    const m = modelEntry(list, id);
    return m ? m.label : String(id).split('::').pop();
  };
  // The efforts on offer for a model (null: the default model). Without a list there are none to go by.
  const effortsOf = (list, model) => {
    if (!list) return [];
    if (!model) return list.efforts || [];
    const m = modelEntry(list, model);
    return m ? m.efforts : list.custom || [];
  };

  // ---------- the agent's own skills ($) ----------

  // The $ menu offers the skills of the agent on screen: Hermes' from its server, Claude Code's and Codex's from their
  // own setup. Main keeps each list a minute; refresh (the menu opened) asks again for one older than ten seconds.
  async function loadOwnSkills(id = currentAgent(), refresh = false) {
    if (P.unavailable || !agentIds().includes(id) || P.loadingSkills.has(id)) return;
    P.loadingSkills.add(id);
    if (id === currentAgent()) showOwnSkills();
    try {
      P.ownSkills[id] = await api('agentOwnSkills', id, { refresh });
    } catch (err) {
      if (!P.ownSkills[id]) P.ownSkills[id] = { skills: [], error: { code: 'unknown', detail: String((err && err.message) || err) } };
    } finally {
      P.loadingSkills.delete(id);
    }
    if (id === currentAgent()) showOwnSkills();
  }

  function showOwnSkills() {
    if (!chat) return;
    const id = currentAgent();
    const name = agentName(id);
    const entry = P.ownSkills[id] || null;
    // The chips in the input and the chat need the list too, so one not asked for yet is asked for now. Until it is
    // there it counts as loading: the chips stay as they are.
    if (!entry && !P.unavailable && !P.loadingSkills.has(id)) loadOwnSkills(id);
    const items = entry && Array.isArray(entry.skills) ? entry.skills : [];
    // A list that failed says why, in the agent's words where Rukoo knows the code (offline, not set up).
    const failed = entry && entry.error && !items.length ? describeError(entry.error, name) : null;
    chat.setSkills({
      items,
      loading: (!entry && !P.unavailable) || (P.loadingSkills.has(id) && !items.length),
      error: failed ? (failed.code ? failed.text : t('agent.panel.skillsFailed', { name })) : '',
      heading: t('agent.panel.skillsOf', { name })
    });
    redrawSkillText();
  }

  // The agent's own skills the chat on screen knows, for the chips in what the user sent. None for a turn that started
  // one of Rukoo's skills: its text is that skill's name.
  function chatSkills(item) {
    const entry = P.conv ? P.ownSkills[P.conv.agent] : null;
    return item.action !== 'skill' && entry && Array.isArray(entry.skills) ? entry.skills : [];
  }
  // A list that came in or changed shows in the messages sent before it.
  let shownSkillText = '';
  function redrawSkillText() {
    const entry = P.conv ? P.ownSkills[P.conv.agent] : null;
    const key = `${P.conv ? P.conv.id : ''} ${entry && Array.isArray(entry.skills) ? entry.skills.map((k) => k.name).join() : ''}`;
    if (key === shownSkillText) return;
    shownSkillText = key;
    for (const view of views.values()) view.skills?.();
  }

  // refresh: the panel just opened, so main loads a list that is older than half a minute again.
  async function loadModels(id = currentAgent(), refresh = false) {
    if (P.unavailable || !agentIds().includes(id)) return;
    P.loadingModels.add(id);
    try {
      P.models[id] = await api('agentModels', id, { refresh });
    } catch (err) {
      // Main could not answer at all: the menus offer the default and the chat's own choice.
      if (!P.models[id]) P.models[id] = { models: [], efforts: [], custom: [], defaultModel: '', error: { code: 'unknown', detail: String((err && err.message) || err) } };
    } finally {
      P.loadingModels.delete(id);
    }
    if (id === currentAgent()) refreshPickers();
  }

  function modelItems() {
    const list = P.models[currentAgent()] || null;
    const { model } = choiceOf();
    const fallback = list && (list.defaultModel || (list.models.find((m) => m.isDefault) || {}).id);
    const items = [{ heading: t('agent.model.title') }, { id: '', label: t('agent.model.default'), tag: fallback ? modelName(list, fallback) : '', checked: !model }];
    const rows = list ? [...list.models] : [];
    // The chat's model stays in the menu when the list no longer has it (or could not load).
    if (model && !rows.some((m) => m.id === model)) rows.push({ id: model, label: modelName(list, model) });
    let group = null;
    for (const m of rows) {
      const name = m.route ? t('agent.model.routes') : m.group || '';
      if (name !== group) {
        items.push(name ? { heading: name } : { separator: true });
        group = name;
      }
      items.push({ id: m.id, label: m.label, tag: m.tag || '', title: m.id, checked: m.id === model });
    }
    if (!list && P.loadingModels.has(currentAgent())) items.push({ id: '', label: t('agent.model.loading'), disabled: true });
    else if (list && list.error) items.push({ separator: true }, { id: '', label: t('agent.model.failed'), title: list.error.detail || '', disabled: true });
    return items;
  }

  function effortItems() {
    const list = P.models[currentAgent()] || null;
    const { model, effort } = choiceOf();
    const levels = [...effortsOf(list, model)];
    if (effort && !levels.includes(effort)) levels.push(effort);
    // What Default comes down to, where the agent says so (Codex, per model).
    const own = modelEntry(list, model || (list && list.defaultModel));
    const items = [{ heading: t('agent.effort.title') }, { id: '', label: t('agent.effort.default'), tag: own && own.defaultEffort ? effortName(own.defaultEffort) : '', checked: !effort }];
    if (levels.length) items.push({ separator: true });
    for (const e of levels) items.push({ id: e, label: effortName(e), checked: e === effort });
    return items;
  }

  // ---------- permission level ----------

  // Main checks the level before every mail action, event and proposal (agents/permission.js); the button by Send
  // shows the level of the chat on screen, its own or the one in Settings.
  const PERMISSIONS = ['ask', 'auto', 'full'];
  const PERMISSION_ICONS = { ask: 'shield', auto: 'shield-half', full: 'shield-off' };
  const defaultPermission = () => (P.config && PERMISSIONS.includes(P.config.permission) ? P.config.permission : 'ask');
  // Claude Code's and Codex's calls are bound to their chat, so a chat of theirs can have any level. Hermes and
  // OpenClaw name the chat in each call themselves, and main gives their chats no looser level than Settings
  // (hub.js BOUND_AGENTS, permissionFor()): the menu doesn't offer one, and one saved before counts as Default.
  const BOUND_AGENTS = new Set(['claude', 'codex']);
  const capped = (level) => !BOUND_AGENTS.has(currentAgent()) && PERMISSIONS.indexOf(level) > PERMISSIONS.indexOf(defaultPermission());
  const ownPermission = () => {
    const own = choiceOf().permission;
    return own && !capped(own) ? own : null;
  };
  const permissionOf = () => ownPermission() || defaultPermission();
  const permissionName = (level) => t(`agent.permission.${level}`);

  function permissionItems() {
    const own = ownPermission();
    const items = [
      { heading: t('agent.permission.title') },
      { id: '', label: t('agent.permission.default'), tag: permissionName(defaultPermission()), checked: !own },
      { separator: true }
    ];
    for (const level of PERMISSIONS) {
      const off = capped(level);
      items.push({
        id: level,
        label: permissionName(level),
        sub: off ? t('agent.permission.onlyInSettings', { name: agentName(currentAgent()) }) : t(`agent.permission.${level}Hint`),
        icon: PERMISSION_ICONS[level],
        checked: level === own,
        disabled: off
      });
    }
    return items;
  }

  // The buttons show the chat on screen: its model (else the default in Settings) and its effort (else only the
  // icon). The effort menu goes away for a model without effort levels.
  function refreshPickers() {
    if (!chat) return;
    const id = currentAgent();
    const blocked = P.unavailable || BLOCKED.has(stateOf(id));
    const list = P.models[id] || null;
    // The panel shows this agent: its list loads now (also after a pick of another agent or chat).
    if (!list && !blocked && !P.loadingModels.has(id) && ctx.agentOpen()) loadModels(id);
    const { model, effort } = choiceOf();
    const shown = model || (list && list.defaultModel) || '';
    const modelText = shown ? modelName(list, shown) : t('agent.model.defaultButton');
    chat.setPicker('model', { text: modelText, label: t('agent.model.button', { model: modelText }), items: modelItems, hidden: blocked });
    chat.setPicker('effort', {
      text: effort ? effortName(effort) : '',
      label: t('agent.effort.button', { effort: effort ? effortName(effort) : t('agent.effort.default') }),
      items: effortItems,
      hidden: blocked || (!effort && !effortsOf(list, model).length),
      meter: effort && effort in METER ? METER[effort] : null
    });
    const level = permissionOf();
    chat.setPicker('permission', {
      text: level === 'full' ? t('agent.permission.fullShort') : permissionName(level),
      label: t('agent.permission.button', { level: permissionName(level) }),
      items: permissionItems,
      hidden: blocked,
      icon: PERMISSION_ICONS[level],
      tone: level === 'full' ? 'warn' : ''
    });
  }

  function pickChoice(kind, value) {
    const list = P.models[currentAgent()] || null;
    const next = { ...choiceOf(), [kind]: value || null };
    // An effort the new model does not offer goes back to the default.
    if (kind === 'model' && next.effort && list && !list.error && !effortsOf(list, next.model).includes(next.effort)) next.effort = null;
    if (!P.conv) {
      P.draft = next;
      return refreshPickers();
    }
    // A model or effort applies from the next turn; a turn that runs now keeps what it started with. The level counts
    // from the agent's next action, and goes alone, so the model and effort stay as main has them.
    const conv = P.conv;
    Object.assign(conv, next);
    refreshPickers();
    api('agentSetChoice', conv.id, kind === 'permission' ? { permission: next.permission } : next)
      .then((saved) => {
        if (saved && P.conv && P.conv.id === conv.id) {
          Object.assign(P.conv, saved);
          refreshPickers();
        }
      })
      .catch((err) => {
        showError(err);
        // Main kept what the chat had: show that again.
        api('agentGet', conv.id)
          .then((c) => {
            if (c && P.conv && P.conv.id === c.id) {
              Object.assign(P.conv, { model: c.model || null, effort: c.effort || null, permission: c.permission || null });
              refreshPickers();
            }
          })
          .catch(() => {});
      });
  }

  // ---------- following the email on screen ----------

  function currentEmail() {
    // In meeting mode the chat is about the meeting on screen, not the email that stays open behind it.
    if (ctx.meetingMode && ctx.meetingMode()) return null;
    const m = S.message;
    return m && m.id && m.id === S.selectedId && !m.error && m.role !== 'drafts' ? m : null;
  }
  // Changes once more when the full message arrives, with its Message-ID and unsubscribe details.
  const emailKey = (m) => (m ? `${m.id}|${m.html === null || m.html === undefined ? '' : 'full'}` : '');
  const messageRef = (m) => ({
    id: m.id,
    messageId: m.messageId || null,
    accountId: m.accountId || null,
    subject: m.subject || '',
    from: m.from ? { name: m.from.name || '', address: m.from.address || '' } : null,
    date: m.date || null,
    // Keeps /unsubscribe available once the chat exists.
    unsubscribe: Boolean(m.unsubscribe)
  });

  // turn: the latest request for what the panel shows; a slower one that lost the race does nothing.
  // lookups: only the searches for the chat of the email on screen.
  let turn = 0;
  let lookups = 0;
  // started: the chats the user started here. A lookup that was out when one started leaves that chat on screen,
  // since showing it doesn't count as a newer request.
  let started = 0;
  // The last time sync() went to find the chat for the email on screen, until that chat shows: "Add to chat" waits
  // for it, so it checks the emails against the chat they land in. Read before this, the panel may still show the
  // chat of an email from before it was opened.
  let looking = Promise.resolve();
  // The meeting on screen in meeting mode: {id, title, eventId}, or null.
  const currentMeeting = () => (ctx.currentMeeting ? ctx.currentMeeting() : null);

  function sync() {
    const before = P.emailKey;
    const run = ctx.meetingMode && ctx.meetingMode() ? syncMeeting() : syncEmail();
    if (P.emailKey !== before) looking = run.catch(() => {});
    return run;
  }
  async function syncEmail() {
    if (!ctx.agentOpen()) return;
    const m = currentEmail();
    const key = emailKey(m);
    if (key === P.emailKey) return;
    const otherEmail = (m ? m.id : null) !== P.emailId;
    P.emailKey = key;
    P.emailId = m ? m.id : null;
    // Goes by the selection, not the loaded message: an email from another folder can take a while to
    // load, and the chat it was opened from stays on screen meanwhile.
    if (P.keepFor && S.selectedId !== P.keepFor) P.keepFor = null;
    if (otherEmail) {
      P.detached = false;
      P.related = null;
      P.draft = { model: null, effort: null };
    }
    // Without an email, or one opened from this chat, the conversation on screen stays.
    if (!m || P.keepFor || (P.detached && !otherEmail)) {
      // A lookup for the email before must not replace the chat that is kept now (a chat being opened
      // from history keeps going).
      if (P.keepFor) lookups++;
      if (!P.conv) renderEmpty();
      return refreshComposer();
    }
    const mine = ++turn;
    const look = ++lookups;
    const began = started;
    let found = null;
    let related = null;
    try {
      found = await api('agentFindFor', { id: m.id, messageId: m.messageId || null, accountId: m.accountId || null });
    } catch (_) {
      found = null;
    }
    // No chat of its own: an earlier email in its thread may have one to continue. The full message has the
    // References header; main has In-Reply-To in its cache.
    if (!(found && found.id)) {
      try {
        related = await api('agentRelatedFor', { id: m.id, messageId: m.messageId || null, accountId: m.accountId || null, inReplyTo: m.inReplyTo || null, references: m.references || [], date: m.date || null });
      } catch (_) {
        related = null;
      }
    }
    if (mine !== turn || look !== lookups) return;
    // The user started a chat about this email while main was looking: it stays, also over an older chat main found.
    // Without P.conv it is still being created and shows when it is.
    if (started !== began && (!P.conv || (P.conv.message && P.conv.message.id === m.id))) return refreshComposer();
    if (found && found.id) {
      if (P.conv && P.conv.id === found.id) return refreshComposer();
      return openConversation(found.id);
    }
    P.related = related && related.id ? { ...related, for: m.id } : null;
    if (P.conv && !otherEmail && P.conv.message && P.conv.message.id === m.id) return refreshComposer();
    // A chat about another email keeps running in the background; this email starts fresh.
    showEmpty();
  }

  // In meeting mode the panel follows the meeting the way it follows an email: its newest chat, or a new one about it.
  // emailKey then names the meeting, so going back to Mail follows the email again.
  async function syncMeeting() {
    if (!ctx.agentOpen()) return;
    const m = currentMeeting();
    const key = m ? `meeting:${m.id}` : 'meeting:';
    if (key === P.emailKey) return refreshComposer();
    P.emailKey = key;
    P.emailId = null;
    P.keepFor = null;
    P.detached = false;
    P.related = null;
    P.draft = { model: null, effort: null };
    // Without a meeting the chat on screen stays, as without an email.
    if (!m) {
      if (!P.conv) renderEmpty();
      return refreshComposer();
    }
    if (P.keepMeeting === m.id && P.conv && P.conv.meeting && P.conv.meeting.id === m.id) {
      P.keepMeeting = null;
      return refreshComposer();
    }
    P.keepMeeting = null;
    const mine = ++turn;
    const look = ++lookups;
    const began = started;
    let found = null;
    try {
      found = await api('agentFindForMeeting', m.id);
    } catch (_) {
      found = null;
    }
    if (mine !== turn || look !== lookups) return;
    // As for an email: a chat about this meeting the user started meanwhile stays.
    if (started !== began && (!P.conv || (P.conv.meeting && P.conv.meeting.id === m.id))) return refreshComposer();
    if (found && found.id) {
      if (P.conv && P.conv.id === found.id) return refreshComposer();
      return openConversation(found.id);
    }
    if (P.conv && P.conv.meeting && P.conv.meeting.id === m.id) return refreshComposer();
    showEmpty();
  }

  async function openConversation(id) {
    const mine = ++turn;
    let conv = null;
    try {
      conv = await api('agentGet', id);
    } catch (err) {
      return showError(err);
    }
    if (mine !== turn || !conv) return;
    showConversation(conv);
  }

  function showConversation(conv) {
    P.conv = { ...conv, items: (conv.items || []).map((i) => ({ ...i })) };
    renderHead();
    renderTranscript();
    refreshComposer();
    watch();
  }

  function showEmpty() {
    turn++;
    P.conv = null;
    renderHead();
    renderEmpty();
    refreshComposer();
    watch();
  }

  // Only before the chat starts: the email went to the agent with the first message, so a chat keeps it.
  function detach() {
    if (P.conv) return;
    P.detached = true;
    showEmpty();
    chat.focus();
  }

  function attach() {
    P.detached = false;
    showEmpty();
    chat.focus();
  }

  // Picks up the chat about an earlier email in this one's thread. It keeps its own email; main notes this one,
  // which goes to the agent with the next message, and shows that chat when this email opens again.
  let continuing = false;
  async function continueChat(offer) {
    const m = currentEmail();
    if (!m || continuing) return;
    continuing = true;
    const mine = ++turn;
    let conv = null;
    try {
      conv = await api('agentContinue', offer.id, messageRef(m));
    } catch (err) {
      showError(err);
    } finally {
      continuing = false;
    }
    if (mine !== turn || !conv) return;
    P.related = null;
    P.detached = false;
    // The chat goes on with its own model and effort; what was picked for a new one is dropped.
    P.draft = { model: null, effort: null };
    showConversation(conv);
    chat.focus();
  }

  function pickAgent(id) {
    if (!agentIds().includes(id)) return;
    // A model or effort picked for one agent means nothing to another; the permission level does.
    if (id !== currentAgent()) P.draft = { model: null, effort: null, permission: P.draft.permission || null };
    P.agentId = id;
    // A conversation stays with its agent; another agent starts a new one about the same email.
    if (P.conv && P.conv.agent !== id) return showEmpty();
    renderHead();
    if (!P.conv) renderEmpty();
    refreshComposer();
  }

  // ---------- sending ----------

  async function ensureConversation() {
    if (P.conv) return P.conv;
    started++;
    const meeting = contextMeeting();
    const m = meeting ? null : contextEmail();
    // The level the menu shows: one picked for Claude Code that a Hermes chat can't have stays behind.
    const conv = await api('agentCreate', {
      agent: P.agentId,
      message: m ? messageRef(m) : null,
      meeting: meeting ? { id: meeting.id, title: meeting.title || '', eventId: meeting.eventId || null } : null,
      model: P.draft.model,
      effort: P.draft.effort,
      permission: ownPermission()
    });
    P.draft = { model: null, effort: null };
    showConversation(conv);
    return P.conv;
  }

  // skill: the name of a skill to start. Main writes that message from the skill itself.
  let sending = false;
  // The files or emails main says are no longer there ({kind, ids}, from missingError in hub.js), or null.
  const missingOf = (why) => {
    const m = /^(file|email)-missing: (\[[\s\S]*\])$/.exec(why);
    if (!m) return null;
    try {
      const ids = JSON.parse(m[2]);
      return Array.isArray(ids) ? { kind: m[1], ids: ids.map(String) } : null;
    } catch (_) {
      return null;
    }
  };

  // The files and emails the user attached go along with whatever is sent, and a message can be those alone. Decided
  // and taken here, before anything waits, so the chat, the text and the attachments are the ones on screen. false:
  // not sent (one is still going out, a file is still on its way, or there is nothing to send); the composer keeps
  // its text then. mode, while the agent works: 'steer' or 'queue' (main's send()); a quick action or skill waits.
  function send(text, action = null, skill = null, mode = null) {
    const body = String(text || '').trim();
    if (sending || holding() || (!body && !action && !skill && !hasAttachments())) return false;
    sending = true;
    clearTurnError();
    const attached = P.attach;
    P.attach = { files: [], emails: [] };
    showAttachments();
    const busy = Boolean(P.conv && P.conv.status === 'running');
    deliver({ body, action, skill, attached, mode: busy ? (action || skill ? 'queue' : mode || 'steer') : null }).finally(() => (sending = false));
    return true;
  }

  async function deliver({ body, action, skill, attached, mode = null }) {
    // Not sent: the chips come back, before anything the user attached meanwhile.
    const restore = () => {
      P.attach = { files: [...attached.files, ...P.attach.files], emails: [...attached.emails, ...P.attach.emails] };
      showAttachments();
    };
    try {
      const conv = await ensureConversation();
      const message = action ? { text: action.prompt, action: action.action, display: action.label } : skill ? { skill } : { text: body };
      await api('agentSend', conv.id, { ...message, files: attached.files.map((f) => f.id), emails: attached.emails.map((m) => m.id), ...(mode ? { mode } : {}) });
      follow(true);
    } catch (err) {
      const why = String((err && err.message) || '');
      // Main names what is gone: those chips go, and every other one stays, wherever its email is.
      const gone = missingOf(why);
      if (gone && gone.kind === 'file') {
        attached.files = attached.files.filter((f) => !gone.ids.includes(f.id));
        toast(t('agent.files.gone', { count: gone.ids.length }), 6000);
      } else if (gone) {
        attached.emails = attached.emails.filter((m) => !gone.ids.includes(m.id));
        toast(t('agent.files.emailGone'), 6000);
      }
      restore();
      if (gone) {
        if (!action && !skill && chat && !chat.getValue()) chat.setValue(body);
        return;
      }
      if (skill && /skill-missing/.test(why)) {
        toast(t('agent.panel.skillMissing', { name: skill }), 6000);
        loadSkills();
        return;
      }
      if (!action && !skill && chat && !chat.getValue()) chat.setValue(body);
      showError(err);
    }
  }

  // A quick action, or a skill: a quick action keeps its command when a skill has the same name. Like send, false
  // when it does not start. While a file is on its way none does, and the user hears why.
  function runAction(name) {
    if (holding()) {
      toast(t('agent.panel.waitFiles'));
      return false;
    }
    // In a meeting chat its own actions come first, before a shortcut with the same name.
    const meetingAction = contextMeeting() ? meetingActionByKey(name) : null;
    if (meetingAction) return send(meetingAction.label, meetingAction);
    const action = shortcutByKey(P.shortcuts, name);
    if (action) {
      if (action.needsEmail && !contextEmail()) {
        toast(t('agent.panel.needsEmail'));
        return false;
      }
      return send(action.label, action);
    }
    const skill = P.skills.find((s) => s.name === name);
    return skill ? send(`/${skill.name}`, null, skill.name) : false;
  }

  // What the input sends with Enter or the Send button. A message that is just one of the offered commands
  // ("/summary", "/shared-name") runs it, the same as picking it from the menu; anything else is chat text. mode:
  // Steer or Queue while the agent works (chatComposer); a command then waits for the turn to end.
  function submit(text, mode = null) {
    const m = /^\/([\w-]+)$/.exec(String(text || '').trim());
    const cmd = m && commands().find((c) => String(c.name).toLowerCase() === m[1].toLowerCase());
    if (cmd) return runAction(cmd.name);
    return send(text, null, null, mode);
  }

  function showError(err) {
    const { text, detail } = describeError(err, agentName(currentAgent()));
    toast(detail ? `${text} (${detail})` : text, 6000);
  }

  // ---------- transcript ----------

  // item id -> { item, update(item), delta(text) }
  const views = new Map();
  let toolRun = null;
  let pendingEl = null;
  let turnErrorEl = null;
  // The messages that wait, at the end of the conversation, and what that list shows (showQueue()).
  let queueEl = null;
  let shownQueue = null;
  // A running tool that waits on the approval card right under it: tool item id -> approval item id.
  // The card says what the tool will do, so the chip stays out of sight until the user allows it.
  const waitsFor = new Map();

  function renderTranscript() {
    views.clear();
    waitsFor.clear();
    toolRun = null;
    pendingEl = null;
    turnErrorEl = null;
    queueEl = null;
    shownQueue = null;
    transcript.replaceChildren();
    pane.classList.remove('is-empty');
    for (const item of P.conv.items) addItem(item);
    showPending();
    showQueue();
    follow(true);
  }

  // The messages that wait (showQueue()) stay at the end of the conversation: what the turn adds goes above them.
  function place(el) {
    transcript.insertBefore(el, queueEl && queueEl.parentNode === transcript ? queueEl : null);
  }

  function wrap(el, item) {
    const box = document.createElement('div');
    box.className = 'ap-item';
    box.dataset.type = item.type;
    box.dataset.id = item.id;
    if (item.interim) box.classList.add('is-interim');
    box.append(el);
    return box;
  }

  // Consecutive tool calls share one row of chips. A call whose card says it all (a successful
  // write_draft, show_plan, ...) or that waits on an approval has no chip; calls in a row with the same
  // label and status share one ("Read a skill ×4").
  function newRun(item) {
    const group = B.toolGroup();
    const box = wrap(elOf(group), item);
    place(box);
    return { group, box, ids: [], chips: new Map() };
  }

  function waiting(item) {
    const approvalId = waitsFor.get(item.id);
    const view = approvalId && views.get(approvalId);
    return Boolean(view && view.item.status !== 'approved');
  }

  function drawRun(run) {
    const runs = [];
    for (const id of run.ids) {
      const view = views.get(id);
      if (!view || cardToolDone(view.item) || waiting(view.item)) continue;
      const props = toolProps(view.item);
      const status = view.item.status || 'done';
      const last = runs[runs.length - 1];
      if (last && last.label === props.label && last.status === status && last.kind === props.kind) last.items.push(view.item);
      else runs.push({ key: id, label: props.label, status, kind: props.kind, rukoo: Boolean(view.item.rukoo), items: [view.item] });
    }
    const keep = new Map();
    const list = runs.map((r) => {
      const props = { label: r.label, status: r.status, kind: r.kind, rukoo: r.rukoo, count: r.items.length, detail: r.items.map((i) => i.detail).filter(Boolean).join('\n') };
      let chip = run.chips.get(r.key);
      if (chip) chip.update(props);
      else chip = B.toolChip(props);
      chip.el.dataset.id = r.key;
      keep.set(r.key, chip);
      return chip;
    });
    run.chips = keep;
    run.group.set(list);
    run.box.hidden = !list.length;
  }

  function addItem(item) {
    if (item.type === 'tool') {
      if (!toolRun) toolRun = newRun(item);
      const run = toolRun;
      run.ids.push(item.id);
      views.set(item.id, { item, run, update: () => drawRun(run) });
      drawRun(run);
      return;
    }
    toolRun = null;
    const view = buildView(item);
    if (!view) return;
    place(wrap(view.el, item));
    views.set(item.id, { item, ...view });
    if (item.type === 'approval') linkApproval(item);
  }

  // A runtime approval right after a tool that was still running (or ended after the card came) is that
  // tool asking for permission.
  function linkApproval(item) {
    if (item.kind !== 'runtime' || !P.conv) return;
    const items = P.conv.items;
    const prev = items[items.findIndex((x) => x.id === item.id) - 1];
    if (!prev || prev.type !== 'tool') return;
    if (prev.status !== 'running' && !(prev.endedAt && item.at && prev.endedAt >= item.at)) return;
    waitsFor.set(prev.id, item.id);
    const view = views.get(prev.id);
    if (view && view.run) drawRun(view.run);
  }

  // The chip waiting on this approval shows again once it is allowed.
  function approvalChanged(approvalId) {
    for (const [toolId, id] of waitsFor) {
      if (id !== approvalId) continue;
      const view = views.get(toolId);
      if (view && view.run) drawRun(view.run);
    }
  }

  function buildView(item) {
    switch (item.type) {
      case 'user': {
        // What went along with the message, as chips without an x.
        const attachments = [...(item.files || []).map((f) => fileChip(f, null)), ...(item.emails || []).map((m) => emailChip(m, null))];
        const labels = { approved: t('agent.items.youApproved'), declined: t('agent.items.youDeclined'), skill: (name) => t('agent.panel.skillChip', { name }) };
        const el = elOf(B.userBubble({ text: item.text || '', action: item.action || null, attachments, labels, skills: chatSkills(item) }));
        // The agent's skills it names are chips, drawn again when the list changes.
        const skills = () => {
          const ub = el.querySelector('.bui-ub');
          if (ub) ub.replaceChildren(...B.skillText(item.text || '', chatSkills(item), labels));
        };
        return { el, skills };
      }
      case 'assistant': {
        const s = B.streamText({ interim: Boolean(item.interim) });
        let shown = item.text || '';
        let finished = false;
        const finish = (i) => {
          if (finished || i.status === 'streaming') return;
          finished = true;
          s.finish();
        };
        s.set(shown);
        finish(item);
        const el = elOf(s);
        return {
          el,
          delta: (text) => {
            shown += text;
            s.append(text);
          },
          update: (next) => {
            // The final item carries the whole text; it corrects anything a lost delta left out.
            if ((next.text || '') !== shown) {
              shown = next.text || '';
              s.set(shown);
            }
            finish(next);
            el.parentElement?.classList.toggle('is-stopped', next.status === 'stopped');
          }
        };
      }
      case 'thinking': {
        const view = B.thinking({ labels: { thinking: t('agent.items.thinking'), thoughtFor: (seconds) => t('agent.items.thoughtFor', { count: Math.max(1, Math.round(seconds)) }) }, startedAt: item.startedAt || item.at });
        const show = (i) => view.update({ text: i.text || '', status: i.status, endedAt: i.endedAt || null });
        show(item);
        return { el: elOf(view), update: show, delta: () => show(views.get(item.id).item) };
      }
      case 'approval': {
        const card = B.approvalCard(localizeApproval(item), {
          labels: {
            approved: t('agent.items.approved'),
            denied: t('agent.items.denied'),
            expired: t('agent.items.expired'),
            waiting: t('agent.items.waiting'),
            showAll: t('agent.approval.showAll'),
            showLess: t('agent.approval.showLess'),
            truncated: (count) => t('agent.approval.truncated', { count, shown: count.toLocaleString(getLocale()) })
          },
          onChoose: (choiceId) => decide(item.id, choiceId)
        });
        return {
          el: elOf(card),
          update: (next) => {
            card.update(localizeApproval(next));
            approvalChanged(next.id);
          }
        };
      }
      case 'plan': {
        const shownTask = (task) => ({ ...task, title: untag(task.title), detail: untag(task.detail), system: untag(task.system), owner: untag(task.owner), due: untag(task.due) });
        const titled = (i) => ({ ...i, title: untag(i.title) || t('agent.items.plan'), tasks: Array.isArray(i.tasks) ? i.tasks.map(shownTask) : i.tasks });
        const rows = B.taskRows(titled(item), {
          labels: { statuses: Object.fromEntries(['proposed', 'todo', 'running', 'done', 'failed', 'skipped'].map((s) => [s, t(`agent.plan.${s}`)])) },
          onOpen: (url) => openUrl(url)
        });
        return { el: elOf(rows), update: (next) => rows.update(titled(next)) };
      }
      case 'sources': {
        const titled = (i) => ({ ...i, title: i.title || t('agent.items.sources') });
        const cards = B.contextCards(titled(item), { labels: { email: t('agent.items.emailSource') }, onOpen: (source) => openSource(source) });
        return { el: elOf(cards), update: (next) => cards.update(titled(next)) };
      }
      case 'draft': {
        const labels = (i) => ({ title: t(`agent.draft.${DRAFT_MODES[i.mode] || 'reply'}`), show: t('agent.draft.show'), undo: t('common.actions.undo'), undone: t('agent.draft.undone'), cannotUndo: t('agent.draft.cannotUndo') });
        const card = B.draftCard({ ...item, canUndo: canUndoDraft(item) }, { labels: labels(item), onAction: (id) => draftAction(views.get(item.id).item, id) });
        return { el: elOf(card), update: (next) => card.update({ ...next, canUndo: canUndoDraft(next) }) };
      }
      case 'notice':
        return noticeView(item, () => undoNotice(item.id));
      default:
        return null;
    }
  }

  // Main and the adapters write approvals in English; the kind of tool, the field keys and the standard
  // choices let the card say it in the interface language. What the user approves (the values) is shown
  // exactly as the agent sent it; only a proposal loses the tags main keeps for the agent (see untag).
  function localizeApproval(item) {
    const name = agentName(agentIds().includes(item.source) ? item.source : P.conv ? P.conv.agent : currentAgent());
    if (item.kind === 'calendar' && item.calendar && typeof item.calendar === 'object') {
      const card = localizeEvent(item, name);
      if (card) return card;
    }
    const tool = item.tool && typeof item.tool === 'object' ? item.tool : null;
    // A runtime approval shows a tool's input, which runs exactly as written, tags and all.
    const shown = item.kind === 'proposal' ? untag : (v) => v;
    let title = shown(item.title);
    if (tool && APPROVAL_TITLES.has(tool.kind)) title = t(`agent.approval.${tool.kind}`, { name });
    else if (tool && tool.kind === 'mcp' && (tool.server || tool.tool)) title = t('agent.approval.tool', { name, tool: [tool.server, tool.tool].filter(Boolean).join(' · ') });
    else if (tool && tool.name) title = t('agent.approval.tool', { name, tool: mcpLabel(tool.name) || tool.name });
    const fields = (item.fields || []).map((input) => {
      const f = { ...input, label: shown(input.label), value: shown(input.value) };
      if (APPROVAL_FIELDS.has(f.key)) return { ...f, label: t(`agent.approval.field.${f.key}`) };
      // A raw parameter name ("output_mode") reads as words.
      const raw = String(f.label || '');
      return /^[a-z][a-z0-9]*(_[a-z0-9]+)+$/.test(raw) ? { ...f, label: (raw[0].toUpperCase() + raw.slice(1)).replace(/_/g, ' ') } : f;
    });
    const itemDetail = shown(item.detail);
    // A detail that only repeats a field (Claude describes a Read by its path) is left out.
    const detail = itemDetail && fields.some((f) => String(f.value || '').trim() === String(itemDetail).trim()) ? '' : itemDetail;
    return {
      ...item,
      title,
      detail,
      fields,
      choices: (item.choices || []).map((c) => {
        // A runtime approval's choices are always the adapter's standard ones; elsewhere a label the agent
        // wrote itself (propose_action's "Add tasks") stays as it is.
        const key = item.kind === 'runtime' && CHOICES.has(c.id) ? `agent.choices.${c.id}` : choiceKey(c.label);
        return key ? { ...c, label: t(key) } : c;
      })
    };
  }

  // An event an agent asked to add (create_event). Main keeps the card in English for the agent's notes; this one
  // says it in the interface language from what Rukoo will make (item.calendar), which is what approving makes.
  // Without the times it needs, the card stays main's.
  function localizeEvent(item, name) {
    const spec = item.calendar;
    const when = eventWhen(spec);
    if (!when) return null;
    const field = (label, value) => ({ label, value: String(value ?? '') });
    const guests = Array.isArray(spec.guests) ? spec.guests : [];
    const fields = [
      field(t('agent.approval.calendarFieldTitle'), spec.title),
      field(t('agent.approval.calendarFieldWhen'), when),
      field(t('agent.approval.calendarFieldCalendar'), spec.account ? `${spec.calendar} (${spec.account})` : spec.calendar)
    ];
    if (guests.length) fields.push(field(t('agent.approval.calendarFieldGuests'), guests.map((g) => (g.name ? `${g.name} <${g.address}>` : g.address)).join(', ')));
    // A product name, the same in every language.
    if (spec.meet) fields.push(field(t('agent.approval.calendarFieldVideo'), 'Google Meet'));
    if (spec.location) fields.push(field(t('agent.approval.calendarFieldLocation'), spec.location));
    if (spec.description) fields.push(field(t('agent.approval.calendarFieldDescription'), spec.description));
    return {
      ...item,
      // Once answered, the card shrinks to its title, which then names the event.
      title: item.status && item.status !== 'pending' ? t('agent.approval.calendarDecided', { title: String(spec.title || '') }) : t('agent.approval.calendarTitle', { name }),
      detail: guests.length && spec.account ? t('agent.approval.calendarInvites', { address: spec.account }) : '',
      fields,
      choices: (item.choices || []).map((c) => {
        if (c.id === 'approve') return { ...c, label: t('agent.approval.calendarCreate') };
        if (c.id === 'decline') return { ...c, label: t('agent.choices.cancel') };
        return c;
      })
    };
  }

  // When the event takes place, in the interface language. On the event's own clock, with its zone after it, when
  // the agent named another time zone than this computer's. An all-day event's end date is the day after it ends.
  function eventWhen(spec) {
    const day = { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' };
    try {
      if (spec.allDay) {
        const f = new Intl.DateTimeFormat(getLocale(), { ...day, timeZone: 'UTC' });
        const first = Date.parse(`${spec.startDate}T00:00:00Z`);
        const last = Date.parse(`${spec.endDate}T00:00:00Z`) - 86400000;
        if (!Number.isFinite(first) || !Number.isFinite(last)) return '';
        return t('agent.approval.calendarAllDay', { days: last > first ? f.formatRange(first, last) : f.format(first) });
      }
      if (!Number.isFinite(spec.start) || !Number.isFinite(spec.end)) return '';
      const zone = spec.timeZone && spec.timeZone !== Intl.DateTimeFormat().resolvedOptions().timeZone ? spec.timeZone : undefined;
      const text = new Intl.DateTimeFormat(getLocale(), { ...day, hour: '2-digit', minute: '2-digit', timeZone: zone }).formatRange(spec.start, spec.end);
      return zone ? `${text} (${zone})` : text;
    } catch {
      return '';
    }
  }

  // Notices from main are English; errors and mail results carry a code or counts we can translate.
  function localizeNotice(item) {
    const cal = item.calendar;
    if (cal && typeof cal.title === 'string' && (cal.action === 'created' || cal.action === 'failed')) {
      if (cal.action === 'failed') return { ...item, text: t('agent.calendar.failed', { title: cal.title }) };
      // Guests got an invitation: said, as nobody may have seen a card with them (a permission level that never asks).
      const guests = Number(cal.guests) || 0;
      return { ...item, text: guests > 0 ? t('agent.calendar.createdInvited', { title: cal.title, count: guests }) : t('agent.calendar.created', { title: cal.title }) };
    }
    if (item.code && hasKey(`agent.notices.${item.code}`)) {
      const params = ['kept-approval', 'approval-too-long', 'auto-approved'].includes(item.code) ? { title: String((item.params && item.params.title) || '') } : {};
      return { ...item, text: t(`agent.notices.${item.code}`, { name: agentName(P.conv ? P.conv.agent : currentAgent()), ...params }) };
    }
    if (item.code && (CODES.has(item.code) || item.tone === 'error')) return { ...item, text: describeError({ code: item.code }, agentName(P.conv ? P.conv.agent : currentAgent())).text };
    const m = item.mail;
    // Only unsubscribe emails were opened: the user still has to send them.
    if (m && m.action === 'unsubscribe_email' && m.mailed > 0 && m.mailed === m.count && !m.failed) {
      return { ...item, text: m.mailed === 1 && m.to ? t('agent.mail.unsubscribeEmailOne', { address: m.to }) : t('agent.mail.unsubscribeEmailMany', { count: m.mailed }) };
    }
    // Only unsubscribe pages were opened: the user finishes in the browser.
    if (m && m.action === 'unsubscribe_page' && m.opened > 0 && m.opened === m.count && !m.failed) {
      return { ...item, text: m.opened === 1 && m.sender ? t('agent.mail.unsubscribePageOne', { name: m.sender }) : t('agent.mail.unsubscribePageMany', { count: m.opened }) };
    }
    if (m && MAIL_RESULTS.has(m.action) && m.count > 0 && !m.failed && (m.action !== 'move' || m.folder)) {
      return { ...item, text: t(`agent.mail.${m.action}`, { count: m.count, folder: m.folder || '' }) };
    }
    return item;
  }

  // A notice line, with the technical detail of an error in small type under it.
  function noticeView(item, onUndo) {
    const line = B.noticeLine(localizeNotice(item), { labels: { undo: t('common.actions.undo') }, onUndo });
    const el = document.createElement('div');
    const detail = document.createElement('div');
    detail.className = 'ap-detail';
    el.append(elOf(line), detail);
    const show = (i) => {
      detail.textContent = i.tone === 'error' ? String(i.detail || '') : '';
      detail.hidden = !detail.textContent;
    };
    show(item);
    return {
      el,
      update: (next) => {
        line.update(localizeNotice(next));
        show(next);
      }
    };
  }

  function upsertItem(item) {
    const items = P.conv.items;
    const i = items.findIndex((x) => x.id === item.id);
    if (i >= 0) items[i] = item;
    else items.push(item);
    const view = views.get(item.id);
    if (view) {
      view.item = item;
      view.update?.(item);
    } else {
      addItem(item);
    }
    showPending();
    follow();
  }

  // "Clark is working…" between your message and the first sign of life from the agent.
  function showPending() {
    const last = P.conv && P.conv.items[P.conv.items.length - 1];
    const want = Boolean(P.conv && P.conv.status === 'running' && (!last || last.type === 'user'));
    if (want && !pendingEl) {
      pendingEl = document.createElement('div');
      pendingEl.className = 'ap-item ap-pending';
      pendingEl.append(elOf(B.shimmer(t('agent.items.working', { name: agentName(P.conv.agent) }))));
      place(pendingEl);
    } else if (!want && pendingEl) {
      pendingEl.remove();
      pendingEl = null;
    }
    // Last of the turn, before the messages that wait.
    const end = queueEl && queueEl.parentNode === transcript ? queueEl.previousElementSibling : transcript.lastElementChild;
    if (pendingEl && pendingEl !== end) place(pendingEl);
  }

  // A turn that failed without leaving a notice of its own still says why.
  function showTurnError(error) {
    const last = P.conv.items[P.conv.items.length - 1];
    if (last && last.type === 'notice' && last.tone === 'error') return;
    clearTurnError();
    const item = { id: 'turn-error', type: 'notice', text: '', detail: error.detail || '', tone: 'error', code: error.code || 'unknown', undo: null };
    turnErrorEl = wrap(noticeView(item, null).el, item);
    place(turnErrorEl);
    follow();
  }

  // ---------- messages that wait ----------

  // What the user sent while the agent worked and is not in the conversation yet (main's c.queue): at its end, each
  // with when it goes. While the agent works, a queued one can steer now (when the agent can take it), be edited
  // back into the input or be removed. Once the agent is done without sending them (Stop, an error, a restart),
  // they wait for the user: Send now starts one. Drawn again only when it changes, so a status update leaves the focus
  // on its buttons.
  function showQueue() {
    if (!P.conv) return;
    const queue = Array.isArray(P.conv.queue) ? P.conv.queue : [];
    const running = P.conv.status === 'running';
    const steer = canSteer(P.conv.agent);
    const key = JSON.stringify([queue.map((e) => [e.id, e.state, e.fallback, e.text]), running, steer, getLocale()]);
    if (key === shownQueue && (queue.length ? queueEl && queueEl.parentNode === transcript : !queueEl)) return;
    shownQueue = key;
    if (!queue.length) {
      queueEl?.remove();
      queueEl = null;
      return;
    }
    const name = agentName(P.conv.agent);
    const box = document.createElement('div');
    box.className = 'ap-item ap-queue';
    box.setAttribute('role', 'list');
    box.setAttribute('aria-label', t('agent.queue.list'));
    for (const e of queue) {
      const steering = e.state === 'steering';
      const tone = steering ? 'steering' : running ? 'queued' : 'held';
      const status = steering ? t('agent.panel.steerHint', { name }) : !running ? t('agent.queue.held') : e.fallback ? t('agent.queue.fallback', { name }) : t('agent.panel.queueHint', { name });
      const actions = [];
      if (!steering) {
        // While the agent works only a message can steer; a quick action or skill, also one of the agent's own named
        // with $, waits for its own turn.
        if (!running || (steer && !e.action && !e.startsSkill)) actions.push({ id: 'send', label: running ? t('agent.queue.steerNow') : t('agent.queue.sendNow'), icon: 'send', onClick: () => sendQueued(e) });
        if (!e.action || e.action === 'skill') actions.push({ id: 'edit', label: t('agent.queue.edit'), icon: 'pen', onClick: () => editQueued(e) });
        actions.push({ id: 'remove', label: t('agent.queue.remove'), icon: 'x', onClick: () => removeQueued(e) });
      }
      const attachments = [...(e.files || []).map((f) => fileChip(f, null)), ...(e.emails || []).map((m) => emailChip(m, null))];
      const el = elOf(B.queuedBubble({ text: e.text || '', attachments, skills: chatSkills(e), status, tone, actions, labels: { skill: (n) => t('agent.panel.skillChip', { name: n }) } }));
      el.setAttribute('role', 'listitem');
      el.dataset.id = e.id;
      box.append(el);
    }
    // The button that had the focus is drawn again: the same one in the new list takes it.
    const had = queueEl && queueEl.contains(document.activeElement) ? document.activeElement : null;
    const focus = had ? { id: had.closest('[data-id]')?.dataset.id, action: had.dataset.action } : null;
    if (queueEl && queueEl.parentNode === transcript) queueEl.replaceWith(box);
    else transcript.append(box);
    queueEl = box;
    if (focus) box.querySelector(`[data-id="${CSS.escape(focus.id || '')}"] [data-action="${CSS.escape(focus.action || '')}"]`)?.focus();
    follow();
  }

  function sendQueued(e) {
    if (!P.conv) return;
    clearTurnError();
    api('agentSendQueued', P.conv.id, e.id)
      .then(() => follow(true))
      .catch((err) => {
        shownQueue = null;
        showQueue();
        showError(err);
      });
  }

  function removeQueued(e) {
    if (!P.conv) return;
    api('agentUnqueue', P.conv.id, e.id).catch((err) => {
      shownQueue = null;
      showQueue();
      showError(err);
    });
    chat.focus();
  }

  // Back into the input, ahead of what the user is writing, with its files and emails as chips again. Main lets go of
  // the waiting copy once it is out of the queue, so a message whose files or emails don't fit next to what the input
  // holds stays in the queue, with a note why, rather than come back without some of them.
  async function editQueued(e) {
    if (!P.conv) return;
    const files = (e.files || []).length;
    const emails = (e.emails || []).filter((m) => !P.attach.emails.some((x) => sameEmail(x, m))).length;
    if (files > MAX_FILES - P.attach.files.length - P.adding.length || emails > MAX_EMAILS - P.attach.emails.length) {
      // Its Edit button turned itself off when clicked; drawn again, it is on.
      shownQueue = null;
      showQueue();
      toast(t('agent.queue.editFull', { files: MAX_FILES, emails: MAX_EMAILS }), 6000);
      return;
    }
    let back;
    try {
      back = await api('agentUnqueue', P.conv.id, e.id, { restage: true });
    } catch (err) {
      shownQueue = null;
      showQueue();
      return showError(err);
    }
    if (!chat) return;
    const typed = chat.getValue();
    const text = String(back.text || '');
    chat.setValue(typed.trim() ? (text ? `${text}\n\n${typed}` : typed) : text);
    for (const m of back.emails || []) {
      if (P.attach.emails.length < MAX_EMAILS && !P.attach.emails.some((x) => sameEmail(x, m))) P.attach.emails.push(m);
    }
    takeFiles(back.files || []);
    chat.focus();
  }

  function clearTurnError() {
    turnErrorEl?.remove();
    turnErrorEl = null;
  }

  // ---------- empty state ----------

  function renderEmpty() {
    if (P.conv) return;
    views.clear();
    waitsFor.clear();
    toolRun = null;
    pendingEl = null;
    turnErrorEl = null;
    queueEl = null;
    shownQueue = null;
    pane.classList.add('is-empty');
    const id = currentAgent();
    const name = agentName(id);
    const m = contextEmail();
    const meeting = contextMeeting();
    // An email is on screen, but the user left it out: the add chip brings it back.
    const leftOut = !m && P.detached && Boolean(currentEmail());
    const state = stateOf(id);
    const box = document.createElement('div');
    box.className = 'ap-empty';
    const blocked = P.unavailable || BLOCKED.has(state);
    // The user turned agents' access to meetings off (Settings > Meetings): the agent gets nothing from the meeting.
    const meetingsOff = Boolean(S.data && S.data.settings && S.data.settings.meetingAgentsCanRead === false);
    const title = blocked ? t('agent.empty.setupTitle', { name }) : meeting ? t('agent.empty.titleMeeting', { name }) : m ? t('agent.empty.title', { name }) : t('agent.empty.titleNoEmail', { name });
    const body = blocked
      ? t('agent.empty.setupBody', { name })
      : meeting
        ? meetingsOff
          ? t('agent.empty.bodyMeetingOff')
          : t('agent.empty.bodyMeeting')
        : m
          ? t('agent.empty.body', { name })
          : leftOut
            ? t('agent.empty.bodyLeftOut')
            : t('agent.empty.bodyNoEmail', { name });
    const card = B.recommendationCard({ eyebrow: name, title, body, actions: [], icon: B.icon('sparkle', 11) }, () => {});
    box.append(elOf(card));
    if (!blocked && meeting) {
      const chips = elOf(B.suggestionChips(MEETING_ACTIONS.map((a) => ({ id: a.key, label: a.label, icon: a.icon })), (key) => runAction(key)));
      chips.classList.add('ap-actions');
      box.append(chips);
    } else if (!blocked) {
      const list = actionsFor(m, P.shortcuts);
      const chips = B.suggestionChips(
        list.map((a) => ({ id: a.key, label: a.label, icon: a.icon })),
        (key) => runAction(key)
      );
      const row = elOf(chips);
      row.classList.add('ap-actions');
      // Settings → Agents → Shortcuts, where the user picks and orders these and adds their own.
      row.append(elOf(B.suggestionEdit({ label: t('agent.panel.editShortcuts'), index: list.length, onClick: () => ctx.openSettings('shortcuts') })));
      box.append(row);
    }
    // Typing still starts a new chat about this email; the offer only takes over when you pick it.
    const offer = P.related;
    const open = currentEmail();
    if (offer && open && offer.for === open.id) {
      const who = agentName(offer.agent);
      box.append(
        elOf(
          B.offerRow({
            label: t('agent.panel.continueChat'),
            sub: `${who} · ${listTime(offer.updatedAt)}`,
            monogram: { label: [...who][0] || '?', agent: true },
            title: offer.title || null,
            onClick: () => continueChat(offer)
          })
        )
      );
    }
    transcript.replaceChildren(box);
    jump.hidden = true;
  }

  // ---------- actions from the transcript ----------

  function decide(itemId, choiceId) {
    if (!P.conv) return;
    api('agentDecide', P.conv.id, itemId, choiceId).catch((err) => showError(err));
  }

  // Source and plan links: besides http(s) and mailto, an Obsidian note (main's openAgentLink).
  function openUrl(url) {
    if (url) api('openAgentLink', String(url)).catch((err) => showError(err));
  }

  // A source being looked up: finding a moved email can sync a folder first, so clicks meanwhile are ignored.
  let locating = false;

  async function openSource(source) {
    if (source && source.messageId) {
      if (locating) return;
      // The email may have moved since the agent showed it; main finds it again by its Message-ID.
      let id = source.messageId;
      locating = true;
      try {
        id = (await api('agentLocate', { id: source.messageId, messageHeader: source.messageHeader || null, accountId: source.accountId || null })) || null;
      } catch (_) {
        // An older main without agentLocate: try the stored id.
      } finally {
        locating = false;
      }
      if (!id) return toast(t('agent.panel.sourceGone'));
      P.keepFor = id;
      return ctx.openMessage(id);
    }
    if (source && source.url) openUrl(source.url);
  }

  // ---------- draft cards ----------

  // A draft card belongs to one write into one composer: its composerKey is "<composer key>:<write>".
  // Show and Undo act on that composer only. A composer that closed leaves a stored draft behind; main keeps
  // which one on its cards (draftId), and follows it when it is saved again, sent or discarded in any window.
  // write key -> conversation id, so an Undo in the composer reaches a card that is not on screen.
  const writes = new Map();
  let writeCount = 0;
  const composerOf = (writeKey) => String(writeKey || '').split(':')[0];
  const openComposerFor = (item) => {
    const c = S.composer;
    return c && item.composerKey && c.key === composerOf(item.composerKey) ? c : null;
  };
  const canUndoDraft = (item) => {
    const c = openComposerFor(item);
    return Boolean(c && !item.undone && c.canUndoAgent(item.composerKey));
  };

  function refreshDrafts() {
    if (!P.conv) return;
    for (const item of P.conv.items) {
      if (item.type !== 'draft') continue;
      const view = views.get(item.id);
      if (view) view.update?.(view.item);
    }
  }

  // app.js reports every inline composer that closes.
  function composerClosed(c) {
    if (!c || !c.key) return;
    // A reopened draft that is saved again gets a new id; cards that pointed at the old one follow it.
    const was = c.opts && c.opts.mode === 'draft' ? c.opts.id : null;
    api('agentKeepDraft', c.key, c.savedDraftId || null, was).catch(() => {});
    refreshDrafts();
  }

  function draftAction(item, action) {
    const c = openComposerFor(item);
    if (action === 'show') {
      if (c) return c.focus();
      if (item.draftId) return ctx.compose({ mode: 'draft', id: item.draftId });
      if (item.messageRef) {
        P.keepFor = item.messageRef;
        return ctx.openMessage(item.messageRef);
      }
      return toast(t('agent.draft.gone'));
    }
    // The composer announces the undo (agent-undo), which marks the cards; see below.
    if (action === 'undo') {
      if (!c || !c.canUndoAgent(item.composerKey)) {
        refreshDrafts();
        return toast(t('agent.draft.gone'));
      }
      c.undoAgent(item.composerKey);
    }
  }

  // Undo from a card or from the composer's own link: the cards of the writes it took back say so,
  // in main too, so it survives a redraw and a restart.
  document.addEventListener('agent-undo', async (e) => {
    const ids = new Set(((e.detail && e.detail.writeIds) || []).map(String));
    if (!ids.size) return refreshDrafts();
    const byConversation = new Map();
    for (const key of ids) {
      const cid = writes.get(key) || (P.conv && P.conv.id);
      if (cid) byConversation.set(cid, [...(byConversation.get(cid) || []), key]);
    }
    for (const [cid, keys] of byConversation) {
      let items = P.conv && P.conv.id === cid ? P.conv.items : null;
      if (!items) items = await api('agentGet', cid).then((conv) => (conv && conv.items) || [], () => []);
      for (const item of items) {
        if (item.type !== 'draft' || item.undone || !keys.includes(String(item.composerKey))) continue;
        if (P.conv && P.conv.id === cid) {
          const view = views.get(item.id);
          item.undone = true;
          if (view) {
            view.item = { ...view.item, undone: true };
            view.update?.(view.item);
          }
        }
        api('agentPatchItem', cid, item.id, { undone: true }).catch(() => {});
      }
    }
    refreshDrafts();
  });

  function undoNotice(itemId) {
    if (!P.conv) return;
    api('agentUndo', P.conv.id, itemId)
      .then(() => ctx.refresh())
      .catch((err) => showError(err));
  }

  // ---------- chats ----------

  // Several chats can run at once: the user starts one, opens another email, and the first goes on. The bell in the
  // header counts the chats with news, and its menu lists every chat, newest message first, with what each is doing.
  const onScreen = (id) => ctx.agentOpen() && Boolean(P.conv) && P.conv.id === id;
  // News: an answer the user has not seen, or an approval waiting for them. The chat on screen has none.
  const news = () => P.sessions.filter((s) => (s.unread || s.waiting) && !onScreen(s.id));

  function renderNews() {
    const list = news();
    const bell = $('[data-ap="history"]', pane);
    const count = $('.ap-count', bell);
    const tone = list.some((s) => s.waiting) ? 'waiting' : 'unread';
    bell.classList.toggle('has-news', list.length > 0);
    bell.dataset.tone = tone;
    count.hidden = !list.length;
    count.textContent = list.length > 99 ? '99+' : String(list.length);
    const label = list.length ? t('agent.panel.historyNews', { count: list.length }) : t('agent.panel.history');
    bell.title = label;
    bell.setAttribute('aria-label', label);
    // With the panel closed, the title bar button says there is news.
    ctx.agentNews?.(list.length ? tone : null);
  }

  // Tells main which chat is on screen: an answer that comes in it is read, one in any other chat waits in the list.
  function watch() {
    const id = ctx.agentOpen() && P.conv ? P.conv.id : null;
    if (id !== P.watching) {
      P.watching = id;
      api('agentWatch', id).catch(() => {});
    }
    const shown = id && P.sessions.find((s) => s.id === id);
    if (shown) shown.unread = null;
    renderNews();
  }

  // Read again a moment after anything that changes a chat in the list: a turn starts or ends, an approval comes or is
  // answered, a chat is read, named, started or deleted.
  let sessionsTimer = null;
  let chatsMenu = null;
  function loadSessions() {
    clearTimeout(sessionsTimer);
    sessionsTimer = setTimeout(async () => {
      try {
        P.sessions = await api('agentList');
      } catch (_) {
        return;
      }
      renderNews();
      chatsMenu?.update(chatItems());
    }, 120);
  }

  // What a chat is doing, in the order it matters to the user. unread says how the turn ended ('reply' or 'error'), also
  // after a restart, when every chat's status is idle again.
  const chatState = (s) => (s.waiting ? 'waiting' : s.status === 'running' ? 'running' : s.unread === 'error' ? 'error' : s.unread ? 'unread' : 'none');
  const STATE_TEXT = { waiting: 'agent.items.waiting', running: 'agent.panel.chatWorking', unread: 'agent.panel.chatReady', error: 'agent.panel.chatFailed' };
  // The preview starts an answer, which can be Markdown: on one line its list marks, heading marks and bold are noise.
  const plain = (text) => String(text || '').replace(/^\s*(?:[-*+>]|#{1,6}|\d+[.)])\s+/, '').replace(/\*\*|__|`/g, '');

  // Every chat, newest message first (main keeps at most 170): one the bell counts is always in the list, which scrolls.
  function chatItems() {
    const items = P.sessions.map((s) => {
      const state = chatState(s);
      const what = STATE_TEXT[state] ? t(STATE_TEXT[state]) : plain(s.preview);
      return {
        id: s.id,
        label: s.title || t('agent.panel.newChat'),
        tag: listTime(s.lastAt || s.updatedAt),
        sub: what ? `${agentName(s.agent)} · ${what}` : agentName(s.agent),
        status: state,
        checked: Boolean(P.conv && s.id === P.conv.id)
      };
    });
    if (!items.length) items.push({ id: '', label: t('agent.panel.noHistory'), tag: '', disabled: true });
    return items;
  }

  async function showChats(anchor) {
    try {
      P.sessions = await api('agentList');
    } catch (err) {
      return showError(err);
    }
    renderNews();
    chatsMenu = B.glideMenu({ items: chatItems(), anchor, align: 'right', width: Math.min(300, window.innerWidth - 16), onPick: (id) => id && openChat(id) });
  }

  // A chat you chose stays while the email on screen finishes loading.
  function keepChat() {
    const m = currentEmail();
    P.keepFor = m ? m.id : S.selectedId || null;
  }

  // A chat picked from the list opens with its email: the newest email it has, which may have moved since (main finds
  // it again by its Message-ID). The chat shows first; its email follows unless the user went on meanwhile.
  async function openChat(id) {
    keepChat();
    const before = S.selectedId;
    await openConversation(id);
    const conv = P.conv;
    if (!conv || conv.id !== id) return;
    // A chat about a meeting opens that meeting in the meeting view, and stays on screen there.
    if (conv.meeting && conv.meeting.id) {
      P.keepFor = null;
      P.keepMeeting = conv.meeting.id;
      if (ctx.openMeeting) ctx.openMeeting(conv.meeting.id);
      return;
    }
    const refs = [...(conv.continued || [])].reverse().concat(conv.message ? [conv.message] : []);
    if (!refs.length) return;
    let target = null;
    for (const ref of refs) {
      try {
        target = await api('agentLocate', { id: ref.id, messageHeader: ref.messageId || null, accountId: ref.accountId || null });
      } catch (_) {
        target = null;
      }
      if (target) break;
    }
    if (!P.conv || P.conv.id !== id || S.selectedId !== before) return;
    if (!target) return toast(t('agent.panel.sourceGone'));
    if (target === S.selectedId) return;
    P.keepFor = target;
    ctx.openMessage(target);
  }

  // ---------- scrolling ----------

  // Follows the newest text unless you scrolled up to read; then a small arrow takes you back down.
  let stick = true;
  let frame = 0;
  scroller.addEventListener(
    'scroll',
    () => {
      stick = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
      if (stick) jump.hidden = true;
    },
    { passive: true }
  );
  function follow(force = false) {
    if (force) stick = true;
    if (!stick) {
      if (P.conv) jump.hidden = false;
      return;
    }
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      scroller.scrollTop = scroller.scrollHeight;
      jump.hidden = true;
    });
  }

  // ---------- events ----------

  pane.addEventListener('click', (e) => {
    const b = e.target.closest('[data-ap]');
    if (!b || !pane.contains(b)) return;
    switch (b.dataset.ap) {
      case 'history':
        return showChats(b);
      case 'new':
        P.detached = false;
        P.draft = { model: null, effort: null };
        showEmpty();
        return chat.focus();
      case 'close':
        return ctx.closeAgent();
      case 'jump':
        scroller.scrollTo({ top: scroller.scrollHeight, behavior: 'smooth' });
        stick = true;
        jump.hidden = true;
        return;
      case 'setup':
        return ctx.openSettings('agents');
    }
  });

  // Files from Explorer and emails from the list can be dropped anywhere on the panel while it takes messages.
  const MAIL_IDS = 'application/x-rukoo-ids';
  const dragged = (e) => {
    const types = [...((e.dataTransfer && e.dataTransfer.types) || [])];
    return types.includes(MAIL_IDS) ? 'mail' : types.includes('Files') ? 'files' : null;
  };
  const droppable = () => Boolean(chat) && !chat.el.classList.contains('is-disabled');
  // Taken on dragenter as well as dragover. When the element under the pointer changes on the last update before the
  // drop, Chromium sends it a dragenter and no dragover, and a dragenter left alone refuses the drop: the email or file
  // was lost without a word. It changes when the pointer has just reached the panel, when is-drop makes the panel
  // itself the target (styles.css), and when the panel draws its contents again, as it does when the agent's status
  // comes in.
  const accept = (e) => {
    if (!dragged(e) || !droppable()) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    pane.classList.add('is-drop');
  };
  pane.addEventListener('dragenter', accept);
  pane.addEventListener('dragover', accept);
  pane.addEventListener('dragleave', (e) => {
    if (!e.relatedTarget || !pane.contains(e.relatedTarget)) pane.classList.remove('is-drop');
  });
  pane.addEventListener('drop', (e) => {
    pane.classList.remove('is-drop');
    const kind = dragged(e);
    if (!kind || !droppable()) return;
    e.preventDefault();
    if (kind === 'mail') {
      let ids = [];
      try {
        ids = JSON.parse(e.dataTransfer.getData(MAIL_IDS));
      } catch (_) {
        return;
      }
      addEmails(ids);
      return chat.focus();
    }
    // A folder only shows as one while the drop event lasts.
    const items = [...(e.dataTransfer.items || [])].filter((i) => i.kind === 'file');
    const list = [...e.dataTransfer.files].map((file, i) => {
      const entry = items[i] && typeof items[i].webkitGetAsEntry === 'function' ? items[i].webkitGetAsEntry() : null;
      return { file, folder: Boolean(entry && entry.isDirectory) };
    });
    addFiles(list).then(() => chat.focus());
  });
  // A drag that ends anywhere, or is called off, leaves no highlight behind.
  document.addEventListener('dragend', () => pane.classList.remove('is-drop'));
  document.addEventListener('drop', () => pane.classList.remove('is-drop'));

  // Escape closes the panel where it covers the mail; menus and the input handle it first.
  pane.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !e.defaultPrevented && ctx.agentOverlay()) {
      e.preventDefault();
      ctx.closeAgent();
    }
  });

  function onConversation(conv) {
    // Started, deleted, named, read or left unread: the bell and the chat list follow.
    loadSessions();
    if (conv.removed) {
      if (P.conv && P.conv.id === conv.id) showEmpty();
      return;
    }
    if (!P.conv || P.conv.id !== conv.id) return;
    if (!Array.isArray(conv.items)) {
      Object.assign(P.conv, conv, { items: P.conv.items });
      renderHead();
      showQueue();
      return refreshComposer();
    }
    // Merge rather than redraw, so streaming text and the scroll position survive.
    const ids = new Set(conv.items.map((i) => i.id));
    if (P.conv.items.some((i) => !ids.has(i.id))) return showConversation(conv);
    Object.assign(P.conv, conv, { items: P.conv.items });
    for (const item of conv.items) upsertItem({ ...item });
    renderHead();
    showQueue();
    refreshComposer();
  }

  function onItem({ conversationId, item }) {
    // An agent waiting for you in a chat that is not on screen: say so instead of letting it hang.
    if (item.type === 'approval' && item.status === 'pending' && (!ctx.agentOpen() || !P.conv || P.conv.id !== conversationId)) {
      toast(t('agent.panel.needsYou'), { action: { label: t('agent.panel.show'), shortcut: false, run: () => reveal(conversationId) } });
    }
    // A message moves its chat up the list (main's lastMessageAt), and an approval card waits for the user. New text
    // comes as deltas, which leave the order alone.
    if (item.type === 'approval' || item.type === 'user' || item.type === 'assistant') loadSessions();
    if (!P.conv || P.conv.id !== conversationId) return;
    // Main names an untitled chat after its first message without a conversation event; follow suit.
    if (item.type === 'user' && !P.conv.title && item.text) {
      P.conv.title = String(item.text).replace(/\s+/g, ' ').trim().slice(0, 80);
      renderHead();
    }
    upsertItem({ ...item });
  }

  // Main dropped the oldest items of a long chat; only their ids come, not the whole transcript again.
  function onTrim({ conversationId, itemIds }) {
    if (!P.conv || P.conv.id !== conversationId || !Array.isArray(itemIds)) return;
    const gone = new Set(itemIds);
    P.conv.items = P.conv.items.filter((i) => !gone.has(i.id));
    const runs = new Set();
    for (const id of gone) {
      const view = views.get(id);
      if (!view) continue;
      views.delete(id);
      waitsFor.delete(id);
      if (view.run) {
        view.run.ids = view.run.ids.filter((x) => x !== id);
        runs.add(view.run);
      } else transcript.querySelector(`.ap-item[data-id="${CSS.escape(id)}"]`)?.remove();
    }
    for (const run of runs) {
      drawRun(run);
      if (!run.ids.length) {
        run.box.remove();
        if (toolRun === run) toolRun = null;
      }
    }
  }

  function onDelta({ conversationId, itemId, text }) {
    if (!P.conv || P.conv.id !== conversationId) return;
    const view = views.get(itemId);
    if (!view) return;
    view.item.text = (view.item.text || '') + text;
    view.delta?.(text);
    follow();
  }

  function onStatus({ conversationId, status, error }) {
    // Any chat: the list shows which ones are working.
    loadSessions();
    if (!P.conv || P.conv.id !== conversationId) return;
    P.conv.status = status;
    chat.setRunning(status === 'running');
    if (status === 'running') clearTurnError();
    showPending();
    if (status === 'error' && error) showTurnError(error);
    // A turn that ended leaves what waits waiting for the user, or the next one starts.
    showQueue();
  }

  // The chat opens before the panel does. A closed panel still holds the chat it showed last, whose answer may be
  // unread; opened first, the panel would tell main that chat is on screen, and main would mark the answer read (#79).
  // keepChat() goes first as well, so the panel's catch-up with the email on screen leaves the new chat alone.
  async function reveal(conversationId) {
    try {
      if (conversationId) {
        keepChat();
        await openConversation(conversationId);
      }
    } finally {
      // Also when the chat could not be loaded: showError's toast says why.
      ctx.revealAgent();
    }
  }

  // ---------- UI bridge: agents writing into the composer ----------

  // The Message-ID header of the email a draft answers, to recognize a reopened reply draft.
  async function headerOf(id, args) {
    if (args.targetMessageId) return args.targetMessageId;
    if (S.message && S.message.id === id && S.message.messageId) return S.message.messageId;
    return api('get', id).then((m) => (m && m.messageId) || null, () => null);
  }

  // Whether the open composer is the one this write is for:
  // - the same email in exactly the same mode (a reply is not a reply-all, nor a forward);
  // - a reopened draft that answers that email (rewritten in place, so no second draft appears);
  // - for a new email, an open new message that is untouched or this chat's own.
  async function fits(d, id, mode, args, mine, theirs) {
    if (!id) return d.mode === 'new' && (mine || !theirs);
    if (d.mode === 'draft') return (mode === 'reply' || mode === 'replyAll') && Boolean(d.inReplyTo) && sameMessageId(d.inReplyTo, await headerOf(id, args));
    return d.messageRef === id && d.mode === mode;
  }

  const busy = () => Object.assign(new Error('The user is writing another email in the composer.'), { code: 'busy' });

  async function writeDraft(args, attempt = 0) {
    let mode = DRAFT_MODES[args.mode] || (args.messageId ? 'reply' : 'new');
    const id = mode === 'new' ? null : args.messageId || null;
    if (!id) mode = 'new';
    const cid = args.conversationId ? String(args.conversationId) : null;
    const conv = P.conv && P.conv.id === cid ? P.conv : null;
    const name = String(args.agentName || agentName(conv ? conv.agent : currentAgent()));
    // A chat that is not on screen, or one an agent started from elsewhere, never closes what the user is writing.
    // Asked again after each wait, so it sees the user close the panel or switch to another chat meanwhile.
    const isBackground = () => !(P.conv && P.conv.id === cid) || !ctx.agentOpen() || P.conv.origin === 'external';
    const background = isBackground();
    const blocked = () => {
      if (background) toast(t('agent.panel.draftBlocked', { name }), { action: { label: t('agent.panel.show'), shortcut: false, run: () => reveal(cid) } });
      return busy();
    };
    let c = S.composer;
    let envelope = null;
    if (c) {
      const d = c.getDraft();
      // This chat's agent wrote what is there, even if the user changed it since.
      const mine = Boolean(d.agent && (cid && d.agent.conversationId ? d.agent.conversationId === cid : d.agent.name === name));
      // The user's own words, saved or not: what they typed since the composer opened, a draft they reopened,
      // or an agent draft they changed since. An address they are still typing counts too, though it is no
      // recipient yet (and no edit until they finish it).
      const typing = Object.values(d.pendingRecipients || {}).some(Boolean);
      const theirs = typing || (d.agent ? Boolean(d.agent.edited) : Boolean(d.userEdited || d.mode === 'draft'));
      const fit = await fits(d, id, mode, args, mine, theirs);
      // A reopened draft keeps no record of whether it was a reply or a reply-all, so recipients the agent
      // left out are set for the mode it asked for: a private reply never keeps the old Cc.
      if (fit && d.mode === 'draft' && (mode === 'reply' || mode === 'replyAll') && (args.to === undefined || args.cc === undefined)) {
        const m = await api('get', id).catch(() => null);
        const acc = m && ((S.data && S.data.accounts) || []).find((a) => a.id === m.accountId);
        if (!m || !acc) throw Object.assign(new Error('The email this draft answers could not be loaded.'), { code: 'busy' });
        envelope = replyEnvelope(acc, m, mode);
      }
      // Everything above may have waited. If the composer, its contents or the panel changed in the
      // meantime, decide again; nothing below waits before the write.
      const now = S.composer === c ? c.getDraft() : null;
      const pending = (x) => JSON.stringify(x.pendingRecipients || {});
      if (!now || now.revision !== d.revision || pending(now) !== pending(d) || isBackground() !== background) {
        if (attempt < 2) return writeDraft(args, attempt + 1);
        throw busy();
      }
      // A chat that is not on screen leaves the user's words alone even where it would fit; the chat on
      // screen may rewrite them, because the user just asked it to (and Undo brings their text back).
      if (background && theirs) throw blocked();
      if (!fit) {
        // A chat that is not on screen never replaces what is open, even a clean reply the user opened
        // themselves; the chat on screen does, unless the user (or another agent) wrote in it.
        if (background || ((d.dirty || theirs) && !mine)) throw blocked();
        c = null;
        envelope = null;
      }
    }
    if (!c) {
      // Keeps the chat input focused; a composer with changes is saved as a draft first. The email loads first:
      // if the chat went on or off screen meanwhile, nothing opens and the write is decided again.
      c = await ctx.compose({ mode, id, focus: false, still: () => isBackground() === background });
      if (isBackground() !== background) {
        if (attempt < 2) return writeDraft(args, attempt + 1);
        throw busy();
      }
      if (!c || c !== S.composer) throw Object.assign(new Error('The composer did not open.'), { code: 'busy' });
    }
    const writeKey = `${c.key}:${++writeCount}`;
    // Only what the agent gave: a field it left out stays as it is, an empty list clears it. For a reopened
    // draft, each recipient field the agent left out comes from the reply rules instead.
    const content = { html: args.html, text: args.html == null ? args.text : undefined };
    if (envelope) {
      content.to = envelope.to;
      content.cc = envelope.cc;
    }
    for (const field of ['to', 'cc', 'bcc', 'subject']) if (args[field] !== undefined) content[field] = args[field];
    c.setContent(content, { mode: 'replace', by: { name, conversationId: cid }, writeId: writeKey });
    if (cid) writes.set(writeKey, cid);
    refreshDrafts();
    const d = c.getDraft();
    return { ok: true, draft: { key: writeKey, mode: d.mode, to: d.to, cc: d.cc, bcc: d.bcc, subject: d.subject, text: d.text } };
  }

  async function bridge(request) {
    // Main gave up on a request the panel only gets now (it was still loading): leave the composer as it is.
    if (expired(request)) return;
    const { requestId, action, args } = request;
    let ok = true;
    let result = null;
    try {
      if (action === 'writeDraft') result = await writeDraft(args || {});
      else if (action === 'getDraft') result = S.composer ? S.composer.getDraft() : null;
      else if (action === 'openMessage') {
        P.keepFor = args && args.messageId;
        await ctx.openMessage(args.messageId);
        result = { ok: true };
      } else throw Object.assign(new Error(`Unknown request: ${action}`), { code: 'protocol' });
    } catch (err) {
      ok = false;
      result = { code: (err && err.code) || 'unknown', detail: String((err && err.message) || err).slice(0, 300) };
    }
    api('agentUiReply', requestId, ok, result).catch(() => {});
  }

  const onEvent = ({ type, payload }) => {
    if (type !== 'agent' || !payload) return;
    switch (payload.kind) {
      case 'conversation':
        return payload.conversation && onConversation(payload.conversation);
      case 'item':
        return payload.item && onItem(payload);
      case 'delta':
        return onDelta(payload);
      case 'trim':
        return onTrim(payload);
      case 'status':
        return onStatus(payload);
      case 'agents':
        P.status = payload.status || P.status;
        return loadAgents();
      case 'ui':
        return bridge(payload);
      case 'reveal':
        return reveal(payload.conversationId);
    }
  };
  window.mail.on(onEvent);
  // Main held a reveal, a waiting approval or a composer request that came before this; it sends them now.
  api('agentPanelReady').catch(() => {});

  // ---------- start ----------

  localizeChrome();
  renderHead();
  buildComposer();
  renderEmpty();
  refreshComposer();
  loadAgents();
  loadSkills();
  // After a reload main may still think the chat from before is on screen.
  watch();
  loadSessions();
  // Open since the last session: follow the email that is already on screen.
  if (ctx.agentOpen()) sync();

  return {
    // The panel became visible: catch up with the email on screen.
    opened({ focus = false } = {}) {
      // Main hears which chat is on screen only once the panel has caught up with the email on screen: while it was
      // closed it kept the chat from before, which may be about another email, and that chat's reply is still unread.
      sync()
        .catch(() => {})
        .then(() => watch());
      loadSkills();
      loadModels(currentAgent(), true);
      loadOwnSkills(currentAgent());
      if (focus) chat.focus();
      follow(true);
    },
    // The panel was closed: an answer in the chat it showed is news again.
    closed() {
      watch();
    },
    // What is on screen changed (an email, the composer): follow it, and let draft cards check their composer.
    sync() {
      refreshDrafts();
      return sync();
    },
    composerClosed,
    // "Add to chat" in the message and bulk menus: what dropping emails on the panel does, for the keyboard too. The
    // app has opened the panel; the emails go in once it shows the chat for the email on screen, with the same
    // checks and limits as a drop. False when the chat takes no messages now (its agent is not set up).
    async addEmails(ids) {
      await looking;
      if (!droppable()) return false;
      const { added, had } = addEmails(ids);
      // Picked from a menu, nothing new would look like nothing happened: say that they are there already.
      if (!added && had) toast(t('agent.panel.inChatAlready', { count: had }));
      chat.focus();
      return true;
    },
    inputFocused: () => Boolean(chat && chat.el.contains(document.activeElement) && (document.activeElement.isContentEditable || /^(TEXTAREA|INPUT)$/.test(document.activeElement.tagName))),
    // False while the agent is not set up or turned off: the input is hidden then.
    inputAvailable: () => Boolean(chat && !chat.el.classList.contains('is-disabled')),
    inputValue: () => (chat ? chat.getValue() : ''),
    relocalize() {
      localizeChrome();
      buildComposer();
      renderHead();
      if (P.conv) renderTranscript();
      else renderEmpty();
      refreshComposer();
    }
  };
}
