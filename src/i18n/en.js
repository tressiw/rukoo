// English source catalog with translator context.
(function (root) {
  const messages = {
  "errors.invite.unreadable": {
    "message": "Rukoo can't read this calendar file.",
    "description": "An .ics file or an email's calendar part holds no event Rukoo can show. src/main/calendar/invitations.js; src/main/main.js (openIcsFile)"
  },
  "errors.invite.notAnswerable": {
    "message": "Rukoo can't answer this invitation.",
    "description": "The invitation was cancelled, is the user's own, or is not a request. src/main/calendar/invitations.js"
  },
  "errors.invite.noOrganizer": {
    "message": "This invitation doesn't say who sent it, so Rukoo can't answer it.",
    "description": "The invitation has no ORGANIZER to email the answer to. src/main/calendar/invitations.js"
  },
  "errors.invite.noAccount": {
    "message": "None of your accounts is invited, so Rukoo can't send your answer.",
    "description": "An .ics file invites an address none of the user's accounts has. src/main/calendar/invitations.js"
  },
  "errors.invite.irregular": {
    "message": "Some occurrences of this series were moved or cancelled, so Rukoo can't copy it to a calendar.",
    "description": "Add to calendar on a recurring invitation whose occurrences differ from its rule: a copy made from the rule would be wrong. src/main/calendar/invitations.js"
  },
  "errors.invite.cancelled": {
    "message": "This event was cancelled.",
    "description": "Add to calendar on a cancelled event. src/main/calendar/invitations.js"
  },
  "invite.card.label": {
    "message": "Calendar invitation",
    "description": "Accessible name of the card. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.from": {
    "message": "Invitation from {name}",
    "description": "Line above the title. {name}: the organizer. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.updatedFrom": {
    "message": "Updated invitation from {name}",
    "description": "Line above the title for a changed event (SEQUENCE above 0). {name}: the organizer. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.cancelledBy": {
    "message": "Cancelled by {name}",
    "description": "Line above the title of a cancelled event. {name}: the organizer. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.cancelled": {
    "message": "Cancelled",
    "description": "Pill beside a cancelled event, and the line above its title when it names no organizer. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.event": {
    "message": "Event",
    "description": "Line above the title of an event that isn't an invitation, such as an .ics file from a ticket shop. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.replied.accepted": {
    "message": "{name} accepted",
    "description": "Line above the title of a guest's answer to the user's own event. {name}: the guest. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.replied.tentative": {
    "message": "{name} might come",
    "description": "Line above the title of a guest's answer Maybe. {name}: the guest. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.replied.declined": {
    "message": "{name} declined",
    "description": "Line above the title of a guest's answer No. {name}: the guest. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.you": {
    "message": "you",
    "description": "The user in the list of guests, lower case inside the list. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.looking": {
    "message": "Looking in your calendars…",
    "description": "Note while Rukoo looks for the event in the user's calendars. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.viaCalendar": {
    "message": "This event is in your calendar, which sends your answer to the organizer.",
    "description": "Note when the answer goes through Google Calendar or a CalDAV server. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.byEmail": {
    "message": "Your answer goes to {address} by email.",
    "description": "Note when no calendar can answer: Rukoo emails the organizer. {address}: the organizer's email address. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.answeredByEmail": {
    "message": "Your answer went to {address} by email.",
    "description": "Note after the user answered by email; another answer goes the same way. {address}: the organizer's email address. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.organizer": {
    "message": "organizer",
    "description": "After the organizer's name in the list of guests, in brackets and lower case: \"Joris Bakker (organizer)\". Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.inCalendar": {
    "message": "This event is in your calendar.",
    "description": "Note for an event in the user's calendar that needs no answer. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.addedNote": {
    "message": "You added this event to your calendar.",
    "description": "Note after Add to calendar. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.notInCalendar": {
    "message": "This event isn't in your calendars.",
    "description": "Note for an event in none of the user's calendars. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.open": {
    "message": "Open in calendar",
    "description": "Button: shows the event in Rukoo's calendar. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.add": {
    "message": "Add to calendar",
    "description": "Button: opens a menu of the user's calendars to put a copy of the event in. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.added": {
    "message": "Added to {calendar}",
    "description": "Toast after Add to calendar. {calendar}: the calendar's name. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.noCalendar": {
    "message": "There's no calendar to add it to.",
    "description": "Toast for Add to calendar without a writable calendar. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.card.sent": {
    "message": "Your answer went to {address}",
    "description": "Toast after an answer was emailed to the organizer. {address}: their address. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.dialog.title": {
    "message": "Calendar file",
    "description": "Title of the dialog for an .ics file opened with Rukoo or from an attachment. Invitation card above an email that carries a calendar invitation, and in the dialog for an .ics file. src/renderer/invite.js"
  },
  "invite.reply.subject.accepted": {
    "message": "Accepted: {title}",
    "description": "Subject of the answer Yes. {title}: the event's title. The email Rukoo sends to an invitation's organizer when the user answers it and no calendar of theirs can (an iTIP REPLY); in the user's language. src/main/calendar/invitations.js"
  },
  "invite.reply.subject.tentative": {
    "message": "Tentatively accepted: {title}",
    "description": "Subject of the answer Maybe. The email Rukoo sends to an invitation's organizer when the user answers it and no calendar of theirs can (an iTIP REPLY); in the user's language. src/main/calendar/invitations.js"
  },
  "invite.reply.subject.declined": {
    "message": "Declined: {title}",
    "description": "Subject of the answer No. The email Rukoo sends to an invitation's organizer when the user answers it and no calendar of theirs can (an iTIP REPLY); in the user's language. src/main/calendar/invitations.js"
  },
  "invite.reply.body.accepted": {
    "message": "{name} has accepted this invitation: {title}",
    "description": "Text of the answer Yes. {name}: the user. {title}: the event's title. The email Rukoo sends to an invitation's organizer when the user answers it and no calendar of theirs can (an iTIP REPLY); in the user's language. src/main/calendar/invitations.js"
  },
  "invite.reply.body.tentative": {
    "message": "{name} might attend: {title}",
    "description": "Text of the answer Maybe. The email Rukoo sends to an invitation's organizer when the user answers it and no calendar of theirs can (an iTIP REPLY); in the user's language. src/main/calendar/invitations.js"
  },
  "invite.reply.body.declined": {
    "message": "{name} has declined this invitation: {title}",
    "description": "Text of the answer No. The email Rukoo sends to an invitation's organizer when the user answers it and no calendar of theirs can (an iTIP REPLY); in the user's language. src/main/calendar/invitations.js"
  },
  "mailbox.sidebar.accountCount": {
    "message": { "one": "{count} account", "other": "{count} accounts" },
    "description": "Sidebar account switcher: number of accounts below the All accounts label."
  },
  "mailbox.sidebar.draftCount": {
    "message": { "one": "{count} draft", "other": "{count} drafts" },
    "description": "Sidebar Drafts count: accessible name for the count badge."
  },
  "mailbox.sidebar.unreadCount": {
    "message": "{count} unread",
    "description": "Sidebar folder count: accessible name for the unread count badge."
  },
  "reader.unsubscribe.unknownSender": {
    "message": "this sender",
    "description": "Unsubscribe confirmation: fallback when no sender name or address is available."
  },
  "composer.quote.excludedPill": {
    "message": "··· excluded",
    "description": "Composer: collapsed previous-message pill when the quoted email is excluded from sending."
  },
  "agent.actions.brief.description": {
    "message": "Who this is and what you should know",
    "description": "Chat panel slash command menu: one-line description of the brief quick action."
  },
  "agent.actions.brief.label": {
    "message": "Brief me",
    "description": "Chat panel quick action chip: the agent explains who the sender is and what to know before replying."
  },
  "agent.actions.reply.description": {
    "message": "Write a reply in the composer",
    "description": "Chat panel slash command menu: one-line description of the reply quick action."
  },
  "agent.actions.reply.label": {
    "message": "Draft a reply",
    "description": "Chat panel quick action chip and slash command label: the agent writes a reply into the composer."
  },
  "agent.actions.summary.description": {
    "message": "The email and its thread in three bullets",
    "description": "Chat panel slash command menu: one-line description of the summary quick action."
  },
  "agent.actions.summary.label": {
    "message": "Summarize",
    "description": "Chat panel quick action chip: the agent summarizes the email and its thread."
  },
  "agent.actions.tasks.description": {
    "message": "Turn the email into tasks",
    "description": "Chat panel slash command menu: one-line description of the follow-ups quick action."
  },
  "agent.actions.tasks.label": {
    "message": "Plan follow-ups",
    "description": "Chat panel quick action chip: the agent turns the email into follow-up tasks."
  },
  "agent.actions.team.description": {
    "message": "Share the key points with your team",
    "description": "Chat panel slash command menu: one-line description of the tell-the-team quick action."
  },
  "agent.actions.team.label": {
    "message": "Tell the team",
    "description": "Chat panel quick action chip: the agent writes a short message for the team instead of forwarding."
  },
  "agent.actions.triage.description": {
    "message": "Go through the inbox, most urgent first",
    "description": "Chat panel slash command menu: one-line description of the inbox overview quick action."
  },
  "agent.actions.triage.label": {
    "message": "What needs me today?",
    "description": "Chat panel quick action chip when no email is open: the agent goes through the inbox."
  },
  "agent.actions.unsubscribe.description": {
    "message": "Stop mail from this sender",
    "description": "Chat panel slash command menu: one-line description of the unsubscribe quick action."
  },
  "agent.actions.unsubscribe.label": {
    "message": "Unsubscribe",
    "description": "Chat panel quick action chip, shown for newsletters: the agent unsubscribes after approval."
  },
  "agent.actions.update.description": {
    "message": "Record it in your CRM, notes or tasks",
    "description": "Chat panel slash command menu: one-line description of the update-systems quick action."
  },
  "agent.actions.update.label": {
    "message": "Update systems",
    "description": "Chat panel quick action chip: the agent records the email in CRM, notes or task systems."
  },
  "agent.approval.changeFiles": {
    "message": "{name} wants to change files",
    "description": "Chat transcript approval card title: the agent asks to write or edit files. name: agent name."
  },
  "agent.approval.command": {
    "message": "{name} wants to run a command",
    "description": "Chat transcript approval card title: the agent asks to run a shell command. name: agent name."
  },
  "agent.approval.permissions": {
    "message": "{name} wants more permissions",
    "description": "Chat transcript approval card title: the agent asks for extra sandbox permissions (network, writing files). name: agent name."
  },
  "agent.approval.readFile": {
    "message": "{name} wants to read a file",
    "description": "Chat transcript approval card title: the agent asks to read a file on this computer. name: agent name."
  },
  "agent.approval.tool": {
    "message": "{name} wants to use {tool}",
    "description": "Chat transcript approval card title for any other tool. name: agent name; tool: the tool, for example \"todoist · add_task\"."
  },
  "agent.approval.web": {
    "message": "{name} wants to open a web page",
    "description": "Chat transcript approval card title: the agent asks to fetch a web page. name: agent name."
  },
  "agent.approval.field.after": {
    "message": "New text",
    "description": "Chat transcript approval card field label: the text an edit puts in."
  },
  "agent.approval.field.before": {
    "message": "Old text",
    "description": "Chat transcript approval card field label: the text an edit replaces."
  },
  "agent.approval.field.command": {
    "message": "Command",
    "description": "Chat transcript approval card field label: the shell command to run."
  },
  "agent.approval.field.content": {
    "message": "Content",
    "description": "Chat transcript approval card field label: what the agent writes into a file."
  },
  "agent.approval.field.diff": {
    "message": "Changes",
    "description": "Chat transcript approval card field label: the diff of a file change."
  },
  "agent.approval.field.file": {
    "message": "File",
    "description": "Chat transcript approval card field label: the file the agent wants to read or change."
  },
  "agent.approval.field.folder": {
    "message": "Folder",
    "description": "Chat transcript approval card field label: the folder a command runs in."
  },
  "agent.approval.field.path": {
    "message": "Path",
    "description": "Chat transcript approval card field label: a file or folder path."
  },
  "agent.approval.field.pattern": {
    "message": "Pattern",
    "description": "Chat transcript approval card field label: the search pattern of a file search."
  },
  "agent.approval.field.query": {
    "message": "Search",
    "description": "Chat transcript approval card field label: what the agent wants to search for."
  },
  "agent.approval.field.reason": {
    "message": "Reason",
    "description": "Chat transcript approval card field label: why the agent asks."
  },
  "agent.approval.field.url": {
    "message": "Web address",
    "description": "Chat transcript approval card field label: the web page the agent wants to open."
  },
  "agent.approval.showAll": {
    "message": "Show all",
    "description": "Chat transcript approval card: unfolds a long command or file content."
  },
  "agent.approval.showLess": {
    "message": "Show less",
    "description": "Chat transcript approval card: folds a long value again."
  },
  "agent.approval.truncated": {
    "message": { "one": "{shown} more character not shown", "other": "{shown} more characters not shown" },
    "description": "Chat transcript approval card, under a value Rukoo had to shorten (over 20,000 characters). shown: the number, formatted."
  },
  "agent.choices.accept": {
    "message": "Allow",
    "description": "Chat transcript approval button: allow this command or change."
  },
  "agent.choices.acceptForSession": {
    "message": "Allow for this chat",
    "description": "Chat transcript approval button: allow similar requests for the rest of this chat."
  },
  "agent.choices.allow": {
    "message": "Allow",
    "description": "Chat transcript approval button: let the agent do it."
  },
  "agent.choices.always": {
    "message": "Always allow",
    "description": "Chat transcript approval button: allow this from now on."
  },
  "agent.choices.approve": {
    "message": "Approve",
    "description": "Chat transcript approval button for a proposed action."
  },
  "agent.choices.cancel": {
    "message": "Cancel",
    "description": "Chat transcript approval button: cancel the request."
  },
  "agent.choices.decline": {
    "message": "Decline",
    "description": "Chat transcript approval button that turns down a proposed action."
  },
  "agent.choices.deny": {
    "message": "Deny",
    "description": "Chat transcript approval button: do not let the agent do it."
  },
  "agent.choices.once": {
    "message": "Allow once",
    "description": "Chat transcript approval button: allow this one time."
  },
  "agent.choices.session": {
    "message": "Allow for this turn",
    "description": "Chat transcript approval button: allow until the agent finishes this answer."
  },
  "agent.draft.forward": {
    "message": "Wrote a forward",
    "description": "Chat transcript draft card title: the agent wrote a forwarded email."
  },
  "agent.draft.gone": {
    "message": "That draft is no longer open.",
    "description": "Toast when you use Show or Undo on a draft card but the composer was closed."
  },
  "agent.draft.cannotUndo": {
    "message": "Can't undo this draft anymore",
    "description": "Tooltip on the disabled Undo button of a draft card: its composer closed or the agent wrote again after your edits."
  },
  "agent.draft.new": {
    "message": "Wrote a new email",
    "description": "Chat transcript draft card title: the agent wrote a new email."
  },
  "agent.draft.reply": {
    "message": "Wrote a reply",
    "description": "Chat transcript draft card title: the agent wrote a reply in the composer."
  },
  "agent.draft.replyAll": {
    "message": "Wrote a reply to all",
    "description": "Chat transcript draft card title: the agent wrote a reply to all."
  },
  "agent.draft.show": {
    "message": "Show",
    "description": "Chat transcript draft card button that shows the draft in the composer."
  },
  "agent.draft.undone": {
    "message": "Undone",
    "description": "Chat transcript draft card: state after you undid the draft."
  },
  "agent.empty.body": {
    "message": "Looks things up and writes the draft. Sending stays with you.",
    "description": "Chat panel empty state line under the heading while an email is open."
  },
  "agent.empty.bodyLeftOut": {
    "message": "Add the email to work on it, or start with what needs you today.",
    "description": "Chat panel empty state line after you left the open email out of the new chat; Add this email is the chip above the input (renderEmpty in agent/panel.js)."
  },
  "agent.empty.bodyNoEmail": {
    "message": "Open an email to work on it, or start with what needs you today.",
    "description": "Chat panel empty state line under the heading without an open email."
  },
  "agent.empty.setupBody": {
    "message": "Rukoo needs a few details before it can reach {name}.",
    "description": "Chat panel empty state line when the agent is not set up; the setup link is below."
  },
  "agent.empty.setupTitle": {
    "message": "Set up {name} to start",
    "description": "Chat panel empty state heading when the agent is not set up or installed."
  },
  "agent.empty.title": {
    "message": "What should {name} do with this email?",
    "description": "Chat panel empty state heading while an email is open; name is the agent."
  },
  "agent.empty.titleNoEmail": {
    "message": "Ask {name} about your mail",
    "description": "Chat panel empty state heading without an open email; name is the agent."
  },
  "agent.effort.button": {
    "message": "Effort: {effort}",
    "description": "Chat panel input: tooltip and accessible name of the effort button beside the model button; effort is the chosen level or Default (refreshPickers in agent/panel.js)."
  },
  "agent.effort.default": {
    "message": "Default",
    "description": "Chat panel effort menu: first row, the agent decides how much effort to spend; also the effort button's name while nothing is picked (effortItems and refreshPickers in agent/panel.js)."
  },
  "agent.effort.high": {
    "message": "High",
    "description": "Chat panel effort menu: effort level high (effortName in agent/panel.js)."
  },
  "agent.effort.low": {
    "message": "Low",
    "description": "Chat panel effort menu: effort level low (effortName in agent/panel.js)."
  },
  "agent.effort.max": {
    "message": "Max",
    "description": "Chat panel effort menu: effort level max, the most effort (effortName in agent/panel.js)."
  },
  "agent.effort.medium": {
    "message": "Medium",
    "description": "Chat panel effort menu: effort level medium (effortName in agent/panel.js)."
  },
  "agent.effort.minimal": {
    "message": "Minimal",
    "description": "Chat panel effort menu: effort level minimal, less than low (effortName in agent/panel.js)."
  },
  "agent.effort.title": {
    "message": "Effort",
    "description": "Chat panel effort menu: heading above the effort levels; effort is how much the model thinks before it answers (effortItems in agent/panel.js)."
  },
  "agent.effort.ultra": {
    "message": "Ultra",
    "description": "Chat panel effort menu: effort level ultra, above max, offered by some Codex models (effortName in agent/panel.js)."
  },
  "agent.effort.xhigh": {
    "message": "Extra high",
    "description": "Chat panel effort menu and button: effort level xhigh, between high and max (effortName in agent/panel.js)."
  },
  "agent.errors.busy": {
    "message": "{name} is still working on the last message.",
    "description": "Chat panel error: a message was sent while the agent was still busy."
  },
  "agent.errors.disabled": {
    "message": "{name} is turned off in Settings.",
    "description": "Chat panel error: the agent is switched off."
  },
  "agent.errors.not-configured": {
    "message": "{name} isn't set up yet.",
    "description": "Chat panel error: the agent has no address or key."
  },
  "agent.errors.not-installed": {
    "message": "{name} isn't installed on this computer.",
    "description": "Chat panel error: the agent program was not found."
  },
  "agent.errors.offline": {
    "message": "Can't reach {name}.",
    "description": "Chat panel error: the agent server or program did not answer; name is the agent."
  },
  "agent.errors.protocol": {
    "message": "{name} sent something Rukoo didn't understand.",
    "description": "Chat panel error: unexpected response from the agent."
  },
  "agent.errors.rate-limited": {
    "message": "{name} is getting too many requests. Try again in a moment.",
    "description": "Chat panel error: the agent answered with a rate limit."
  },
  "agent.errors.spawn-failed": {
    "message": "{name} didn't start.",
    "description": "Chat panel error: the agent program could not be started or stopped unexpectedly."
  },
  "agent.errors.stopped": {
    "message": "Stopped.",
    "description": "Chat panel: shown when the turn was stopped."
  },
  "agent.errors.timeout": {
    "message": "{name} took too long to answer.",
    "description": "Chat panel error: the agent did not answer in time."
  },
  "agent.errors.unauthorized": {
    "message": "{name} didn't accept the key or token.",
    "description": "Chat panel error: authentication with the agent failed."
  },
  "agent.errors.unknown": {
    "message": "Something went wrong.",
    "description": "Chat panel error: fallback when the cause is not known."
  },
  "agent.files.adding": {
    "message": "Adding {name}",
    "description": "Chat panel, accessible name of a file chip while the file is still being read and handed to main; name is the file name (pendingChip in agent/panel.js)."
  },
  "agent.files.blocked": {
    "message": "{name} is a program or script. Rukoo doesn't hand those to agents.",
    "description": "Chat panel notice after you attach a program or script (.exe, .js and the like) to a chat message; name is the file name (takeFiles and refusal in agent/panel.js)."
  },
  "agent.files.emailGone": {
    "message": "An email you added is no longer here. Your message was not sent; the other attachments are still there.",
    "description": "Chat panel notice when you send a message with an email attached that was moved or deleted meanwhile (send in agent/panel.js)."
  },
  "agent.files.folder": {
    "message": "{name} is a folder. Add the files in it instead.",
    "description": "Chat panel notice after you drop a folder on the chat; name is the folder name (addFiles in agent/panel.js)."
  },
  "agent.files.full": {
    "message": "{name} didn't fit: Rukoo already holds as many files as it keeps for unsent messages. Send or remove some first.",
    "description": "Chat panel notice when a file can't be attached because the files waiting in unsent chat messages already take the room Rukoo keeps for them (30 files or 120 MB); name is the file name (refusal in agent/panel.js)."
  },
  "agent.files.gone": {
    "message": {
      "one": "{count} file you attached was no longer ready to send, so your message was not sent. Attach it again; the other attachments are still there.",
      "other": "{count} files you attached were no longer ready to send, so your message was not sent. Attach them again; the other attachments are still there."
    },
    "description": "Chat panel notice when you send a message with files Rukoo let go of (after two hours); only their chips are removed. count: how many (send in agent/panel.js)."
  },
  "agent.files.tooBig": {
    "message": "{name} is larger than 10 MB, so it can't go to the agent.",
    "description": "Chat panel notice after you attach a file over 10 MB to a chat message; name is the file name (takeFiles, addFiles and refusal in agent/panel.js)."
  },
  "agent.files.tooMany": {
    "message": "A message takes up to {count} files.",
    "description": "Chat panel notice when you attach more files to one chat message than it takes; count is the maximum (takeFiles and addFiles in agent/panel.js)."
  },
  "agent.files.tooManyEmails": {
    "message": "A message takes up to {count} emails.",
    "description": "Chat panel notice when you drag more emails onto one chat message than it takes; count is the maximum (addEmails in agent/panel.js)."
  },
  "agent.files.unreadable": {
    "message": "{name} could not be read.",
    "description": "Chat panel notice when Rukoo could not read a file you attached; name is the file name (refusal and addFiles in agent/panel.js)."
  },
  "agent.notices.approval-too-long": {
    "message": "Declined without asking: {title}. Part of it is too long to show here, so you could not check it.",
    "description": "Chat transcript info line: Rukoo declined an approval request itself because a value was too long to show in full; title is the request."
  },
  "agent.notices.kept-approval": {
    "message": "Not done yet: {title}. {name} hears about your approval with your next message.",
    "description": "Chat transcript info line: you approved a proposal, but the follow-up turn could not start (for example the app quit). title is the proposal, name the agent."
  },
  "agent.notices.auto-approved": {
    "message": "Allowed without asking: {title}",
    "description": "Chat transcript info line: the agent proposed an action in another system (propose_action) and the chat's permission level is Full permission, so Rukoo let it go ahead without an approval card. title is what the agent said it will do, e.g. \"Add 2 tasks to Todoist\" (tools.js proposeAction, agent/panel.js localizeNotice)."
  },
  "agent.notices.new-session": {
    "message": "{name} started a new session",
    "description": "Chat panel info line (agent/panel.js localizeNotice): Claude Code or Hermes no longer had this chat's session, so it started a new one (claude.js ClaudeAdapter.turn, hermes.js HermesAdapter.session). Rukoo sends that session the email and a recap of the chat. name is the agent name."
  },
  "agent.notices.new-thread": {
    "message": "{name} started a new thread",
    "description": "Chat panel info line (agent/panel.js localizeNotice): Codex could not reopen this chat's thread, so it started a new one (codex.js CodexAdapter.ensureThread). Rukoo sends that thread the email and a recap of the chat. name is the agent name."
  },
  "agent.errors.window": {
    "message": "Rukoo's main window is closed.",
    "description": "Chat panel error: an agent needed the main window, which was closed."
  },
  "agent.items.approved": {
    "message": "Approved",
    "description": "Chat transcript approval card: state after you approved."
  },
  "agent.items.denied": {
    "message": "Declined",
    "description": "Chat transcript approval card: state after you declined or denied."
  },
  "agent.items.emailSource": {
    "message": "Email",
    "description": "Chat transcript: label on a source card that points to an email in Rukoo."
  },
  "agent.items.plan": {
    "message": "Plan",
    "description": "Chat transcript: heading of a plan card when the agent gave it no title."
  },
  "agent.items.sources": {
    "message": "Sources",
    "description": "Chat transcript: heading of the sources card when the agent gave it no title."
  },
  "agent.items.expired": {
    "message": "Expired",
    "description": "Chat transcript approval card: the request ended before you answered."
  },
  "agent.items.thinking": {
    "message": "Thinking",
    "description": "Chat transcript: shimmering label while the agent reasons."
  },
  "agent.items.thoughtFor": {
    "message": "Thought for {count}s",
    "description": "Chat transcript: label after the agent reasoned; count is seconds."
  },
  "agent.items.waiting": {
    "message": "Waiting for you",
    "description": "Chat transcript approval card: state while the agent waits for your answer."
  },
  "agent.items.working": {
    "message": "{name} is working on it...",
    "description": "Chat transcript: shown after you send a message, before the agent responds."
  },
  "agent.items.youApproved": {
    "message": "You approved",
    "description": "Chat transcript: compact line before the title of something you approved."
  },
  "agent.items.youDeclined": {
    "message": "You declined",
    "description": "Chat transcript: compact line before the title of something you declined."
  },
  "agent.mail.unsubscribeEmailMany": {
    "message": "Opened {count} unsubscribe emails. Send them to finish.",
    "description": "Chat transcript notice: senders only offer unsubscribing by email, so Rukoo opened that many prepared emails for you to send."
  },
  "agent.mail.unsubscribeEmailOne": {
    "message": "Opened an unsubscribe email to {address}. Send it to finish.",
    "description": "Chat transcript notice: the sender only offers unsubscribing by email; Rukoo opened a prepared email to address for you to send."
  },
  "agent.mail.unsubscribePageMany": {
    "message": "Opened {count} unsubscribe pages in your browser. Finish there; Rukoo Mail can't tell whether they worked.",
    "description": "Chat transcript notice: the senders' unsubscribe links are web pages, so Rukoo opened that many in the browser; the user may still have to confirm on each."
  },
  "agent.mail.unsubscribePageOne": {
    "message": "Opened the unsubscribe page for {name} in your browser. Finish there; Rukoo Mail can't tell whether it worked.",
    "description": "Chat transcript notice: the sender's unsubscribe link is a web page, so Rukoo opened it in the browser; the user may still have to confirm there. name is the sender's name or address."
  },
  "agent.mail.archive": {
    "message": { "one": "Archived {count} email", "other": "Archived {count} emails" },
    "description": "Chat transcript notice after Rukoo archived mail for an agent; count is the number of emails."
  },
  "agent.mail.mark_read": {
    "message": { "one": "Marked {count} email as read", "other": "Marked {count} emails as read" },
    "description": "Chat transcript notice after Rukoo marked mail as read for an agent."
  },
  "agent.mail.mark_unread": {
    "message": { "one": "Marked {count} email as unread", "other": "Marked {count} emails as unread" },
    "description": "Chat transcript notice after Rukoo marked mail as unread for an agent."
  },
  "agent.mail.move": {
    "message": { "one": "Moved {count} email to {folder}", "other": "Moved {count} emails to {folder}" },
    "description": "Chat transcript notice after Rukoo moved mail for an agent; folder is the folder path."
  },
  "agent.mail.star": {
    "message": { "one": "Starred {count} email", "other": "Starred {count} emails" },
    "description": "Chat transcript notice after Rukoo added a star to mail for an agent."
  },
  "agent.mail.trash": {
    "message": { "one": "Moved {count} email to Trash", "other": "Moved {count} emails to Trash" },
    "description": "Chat transcript notice after Rukoo deleted mail for an agent (moved to the trash)."
  },
  "agent.mail.undo": {
    "message": { "one": "Put {count} email back", "other": "Put {count} emails back" },
    "description": "Chat transcript notice after you undid a mail action from the chat."
  },
  "agent.mail.unstar": {
    "message": { "one": "Removed the star from {count} email", "other": "Removed the star from {count} emails" },
    "description": "Chat transcript notice after Rukoo removed stars for an agent."
  },
  "agent.model.button": {
    "message": "Model: {model}",
    "description": "Chat panel input: tooltip and accessible name of the model button beside the agent picker; model is the model name or Default model (refreshPickers in agent/panel.js)."
  },
  "agent.model.default": {
    "message": "Default",
    "description": "Chat panel model menu: first row, the chat uses the agent's default model from Settings, else the agent's own (modelItems in agent/panel.js)."
  },
  "agent.model.defaultButton": {
    "message": "Default model",
    "description": "Chat panel input: text of the model button while the chat follows the agent's default and Settings name no model (refreshPickers in agent/panel.js)."
  },
  "agent.model.failed": {
    "message": "Could not load the models",
    "description": "Chat panel model menu: disabled row when the agent did not list its models; the menu still offers the default (modelItems in agent/panel.js)."
  },
  "agent.model.loading": {
    "message": "Loading models...",
    "description": "Chat panel model menu: disabled row while the agent's model list loads (modelItems in agent/panel.js)."
  },
  "agent.model.routes": {
    "message": "Model routes",
    "description": "Chat panel model menu: heading above the Hermes model_routes aliases, short names a Hermes server maps to a model (modelItems in agent/panel.js)."
  },
  "agent.model.title": {
    "message": "Model",
    "description": "Chat panel model menu: heading at the top of the menu (modelItems in agent/panel.js)."
  },
  "agent.panel.addContext": {
    "message": "Add this email",
    "description": "Chat panel input: dashed chip in the email chip's place after you left the email out of a new chat; clicking it puts the email back (contextChip and attach in agent/panel.js)."
  },
  "agent.panel.addToChat": {
    "message": "Add to chat",
    "description": "Item in an email's menu (the reader's More button, a right-click on the list) and in the menu for checked emails: adds the email or emails to the message being written in the chat panel, as dragging them onto the panel does, and opens the panel if it is closed (addToChatItem in app.js)."
  },
  "agent.panel.inChatAlready": {
    "message": {
      "one": "That email is in the chat already.",
      "other": "Those emails are in the chat already."
    },
    "description": "Chat panel notice after Add to chat (email menus) when every email picked is already the chat's own email, a newer email the chat was continued from, or added to the message being written; count: how many were picked (addEmails in agent/panel.js)."
  },
  "agent.panel.attach": {
    "message": "Attach files",
    "description": "Chat panel input: tooltip and label of the paperclip button before the email chip, and the title of the file dialog it opens (pickFiles in agent/panel.js, chatComposer in agent/bui.js, agentPickFiles in main.js)."
  },
  "agent.panel.close": {
    "message": "Close chat",
    "description": "Chat panel header: tooltip of the button that closes the panel."
  },
  "agent.panel.continueChat": {
    "message": "Continue the chat about the earlier message",
    "description": "Chat panel empty state: quiet row under the quick actions when the open email has no chat but an earlier email in its thread has one; the agent and the chat's date are under it (renderEmpty and continueChat in agent/panel.js)."
  },
  "agent.panel.dropHere": {
    "message": "Drop files or emails here to add them to your message",
    "description": "Chat panel: text in the dashed area that appears while you drag files from Explorer or emails from the list over the panel (localizeChrome in agent/panel.js)."
  },
  "agent.panel.history": {
    "message": "Recent chats",
    "description": "Chat panel header: tooltip of the bell button that lists the chats, newest message first, while none has news (renderNews in agent/panel.js)."
  },
  "agent.panel.historyNews": {
    "message": "Recent chats, {count} new",
    "description": "Chat panel header: tooltip and accessible name of the bell button while chats have news: an answer you have not seen, or an agent waiting for your approval. count: how many chats (renderNews in agent/panel.js)."
  },
  "agent.panel.chatWorking": {
    "message": "Working…",
    "description": "Chat panel, list of chats under the bell: second line of a chat whose agent is working on an answer, after the agent's name (chatItems in agent/panel.js)."
  },
  "agent.panel.chatReady": {
    "message": "Reply ready",
    "description": "Chat panel, list of chats under the bell: second line of a chat whose agent answered while you were looking at another chat or had the panel closed, after the agent's name. Next to a blue dot (chatItems in agent/panel.js)."
  },
  "agent.panel.chatFailed": {
    "message": "Stopped with an error",
    "description": "Chat panel, list of chats under the bell: second line of a chat whose agent hit an error while you were looking at another chat, after the agent's name. Next to a red dot (chatItems in agent/panel.js)."
  },
  "agent.panel.jump": {
    "message": "Go to the latest message",
    "description": "Chat panel: tooltip of the arrow that scrolls back down after you scrolled up."
  },
  "agent.panel.label": {
    "message": "Agent chat",
    "description": "Chat panel: accessible name of the side panel where you chat with an agent."
  },
  "agent.panel.needsEmail": {
    "message": "Open an email first.",
    "description": "Chat panel toast: an email quick action was picked while no email is attached."
  },
  "agent.panel.removeAttachment": {
    "message": "Remove {name}",
    "description": "Chat panel input: label of the x on a file or email chip you attached to the message; name is the file name or the email subject (fileChip and emailChip in agent/panel.js)."
  },
  "agent.panel.skillMissing": {
    "message": "Rukoo can't find the skill /{name} anymore.",
    "description": "Chat panel toast (panel.js send): you picked a skill from the slash command menu, but its folder is gone or no longer valid. name is the skill name."
  },
  "agent.panel.needsYou": {
    "message": "An agent is waiting for your approval",
    "description": "Toast when an agent asks for approval in a chat that is not on screen."
  },
  "agent.panel.draftBlocked": {
    "message": "{name} has a draft for you. Finish or close the email you're writing first.",
    "description": "Toast when a chat that is not on screen wants to write a draft while you are writing another email. name: agent name."
  },
  "agent.panel.new": {
    "message": "New chat",
    "description": "Chat panel header: tooltip of the plus button that starts a new conversation."
  },
  "agent.panel.newChat": {
    "message": "New chat",
    "description": "Chat panel header: title line for a conversation without a title yet, and history fallback."
  },
  "agent.panel.newerMessage": {
    "message": "Newer message",
    "description": "Chat panel input: grey text in the second email chip, next to the subject of the newer email you continued an earlier chat from, while that email is open (contextChip in agent/panel.js)."
  },
  "agent.panel.newerPending": {
    "message": "You continued this chat from this newer message. It goes to {name} with your next message.",
    "description": "Chat panel input: tooltip of Newer message in the second email chip, before the agent has had that email; name is the agent (contextChip in agent/panel.js)."
  },
  "agent.panel.newerSent": {
    "message": "You continued this chat from this newer message. {name} has it.",
    "description": "Chat panel input: tooltip of Newer message in the second email chip, once a message took that email to the agent; name is the agent (contextChip in agent/panel.js)."
  },
  "agent.panel.noHistory": {
    "message": "No chats yet",
    "description": "Chat panel history menu: shown when there are no earlier conversations."
  },
  "agent.panel.noMatches": {
    "message": "No matching commands",
    "description": "Chat panel slash command menu: shown when no command matches what you typed."
  },
  "agent.panel.editShortcuts": {
    "message": "Edit shortcuts",
    "description": "Chat panel empty state: tooltip and accessible name of the round button after the quick action chips (suggestionEdit in agent/bui.js); it opens Settings > Agents > Shortcuts."
  },
  "agent.panel.skillsLoading": {
    "message": "Loading skills…",
    "description": "Chat panel input, $ menu: shown while Rukoo asks the agent for its own skills (Hermes, Claude Code or Codex)."
  },
  "agent.panel.noSkills": {
    "message": "No skills found",
    "description": "Chat panel input, $ menu: the agent answered, but has no skills installed."
  },
  "agent.panel.skillsFailed": {
    "message": "Couldn't load {name}'s skills",
    "description": "Chat panel input, $ menu: Rukoo could not get the list of the agent's own skills, for a reason it has no sentence for. name: the agent (Hermes, Claude, Codex)."
  },
  "agent.panel.skillsOf": {
    "message": "{name}'s skills",
    "description": "Chat panel input, $ menu: grey heading above the list of the agent's own skills. name: the agent (Hermes, Claude, Codex)."
  },
  "agent.panel.skillChip": {
    "message": "Skill {name}",
    "description": "Chat panel: what a screen reader says for the chip of one of the agent's own skills, in the input and in a sent message. name: the skill as the chip shows it (Babysit, Meeting Prep)."
  },
  "agent.panel.pickAgent": {
    "message": "Choose agent",
    "description": "Chat panel input: label of the agent picker (Hermes, Claude Code, Codex)."
  },
  "agent.panel.placeholder": {
    "message": "Ask {name}...",
    "description": "Chat panel input placeholder without an email; name is the agent name."
  },
  "agent.panel.placeholderEmail": {
    "message": "Ask {name} about this email...",
    "description": "Chat panel input placeholder while an email is attached; name is the agent name."
  },
  "agent.panel.removeContext": {
    "message": "Leave this email out",
    "description": "Chat panel input: tooltip of the x on the email chip of a chat that has not started yet; the chat then starts without the email (contextChip and detach in agent/panel.js)."
  },
  "agent.panel.send": {
    "message": "Send",
    "description": "Chat panel input: label of the button that sends your chat message (not an email)."
  },
  "agent.panel.setUp": {
    "message": "Set up {name}",
    "description": "Chat panel: link-style button that opens Settings, Agents; name is the agent name."
  },
  "agent.panel.show": {
    "message": "Show",
    "description": "Toast action that opens the chat with the waiting approval."
  },
  "agent.panel.sourceGone": {
    "message": "That email is no longer in Rukoo. It may have been deleted, or moved to a folder Rukoo has not downloaded.",
    "description": "Chat panel toast: you clicked a source card for an email that Rukoo can no longer find."
  },
  "agent.panel.stop": {
    "message": "Stop",
    "description": "Chat panel input: label of the button that stops the agent while it works."
  },
  "agent.panel.steer": {
    "message": "Steer",
    "description": "Chat panel input, while the agent works: the way of sending that hands your message to the agent during its current turn, so it can change course. Label of the main part of the split send button and of its menu row (chatComposer in agent/bui.js). Short verb."
  },
  "agent.panel.steerHint": {
    "message": "{name} reads it before its next step",
    "description": "Chat panel, while the agent works: what Steer does, under Steer in the send menu and in the button's tooltip; also the line under a steered message that the agent has not read yet. name is the agent name."
  },
  "agent.panel.queue": {
    "message": "Queue",
    "description": "Chat panel input, while the agent works: the way of sending that holds your message until the agent's current turn is over and then sends it. Menu row in the send menu, and the main button for an agent that can't take a message during its turn. Short verb."
  },
  "agent.panel.queueHint": {
    "message": "Sends when {name} is done",
    "description": "Chat panel, while the agent works: what Queue does, under Queue in the send menu and in the button's tooltip; also the line under a queued message at the end of the conversation. name is the agent name."
  },
  "agent.panel.cantSteer": {
    "message": "{name} can't read messages while it works",
    "description": "Chat panel send menu: why Steer is off for this agent (OpenClaw, or a Hermes server without the steer route); its messages are queued instead. name is the agent name."
  },
  "agent.panel.sendOptions": {
    "message": "Send options",
    "description": "Chat panel input, while the agent works: accessible name and tooltip of the chevron beside the send button, which opens the menu with Steer and Queue."
  },
  "agent.queue.list": {
    "message": "Messages waiting to be sent",
    "description": "Chat panel: accessible name of the list at the end of the conversation with the messages you sent while the agent worked that are not in the conversation yet (showQueue in agent/panel.js)."
  },
  "agent.queue.fallback": {
    "message": "{name} can't read messages while it works. This one sends when {name} is done.",
    "description": "Chat panel: line under a message you meant to steer, which waits instead because the agent can't take a message during its turn. name is the agent name."
  },
  "agent.queue.held": {
    "message": "Not sent yet",
    "description": "Chat panel: line under a waiting message once the agent stopped working without sending it (you pressed Stop, the turn failed, or Rukoo restarted). It waits until you send it, edit it or remove it."
  },
  "agent.queue.sendNow": {
    "message": "Send now",
    "description": "Chat panel: button under a waiting message, while the agent is idle, that sends it as the next message."
  },
  "agent.queue.steerNow": {
    "message": "Steer now",
    "description": "Chat panel: button under a queued message, while the agent works, that hands it to the agent during the current turn instead of after it."
  },
  "agent.queue.edit": {
    "message": "Edit",
    "description": "Chat panel: button under a waiting message that takes it out of the queue and puts its text and attachments back in the input."
  },
  "agent.queue.editFull": {
    "message": "This message keeps waiting: the one you're writing has no room for its attachments. A message takes up to {files} files and {emails} emails.",
    "description": "Chat panel notice after you click Edit under a waiting message whose files or emails don't fit next to those already in the input; the message stays in the queue. files and emails are the maximums per message (editQueued in agent/panel.js)."
  },
  "agent.queue.remove": {
    "message": "Remove",
    "description": "Chat panel: button under a waiting message that takes it out of the queue without sending it."
  },
  "agent.panel.toggle": {
    "message": "Chat with an agent (Ctrl+J)",
    "description": "Title bar button tooltip that opens or closes the chat panel."
  },
  "agent.panel.transcript": {
    "message": "Conversation",
    "description": "Chat panel: accessible name of the list of messages in the conversation."
  },
  "agent.panel.waitFiles": {
    "message": "Wait until the files are added, then send.",
    "description": "Chat panel: why Send is off while a file you dropped, pasted or picked is still being added (tooltip of the Send button), and the notice when you pick a quick action or skill meanwhile (refreshHolding and runAction in agent/panel.js)."
  },
  "agent.permission.title": {
    "message": "Permission level",
    "description": "Chat panel: heading of the permission menu, opened from the shield button beside Send. The level decides which actions of the agent wait for your approval in this chat (permissionItems in agent/panel.js)."
  },
  "agent.permission.button": {
    "message": "Permission level: {level}",
    "description": "Chat panel: tooltip and accessible name of the shield button beside Send. level is the chat's level, e.g. \"Ask\" (refreshPickers in agent/panel.js)."
  },
  "agent.permission.default": {
    "message": "Default",
    "description": "Chat panel permission menu: first row, the chat follows the permission level in Settings > Agents. The level it comes down to shows beside it (permissionItems in agent/panel.js)."
  },
  "agent.permission.ask": {
    "message": "Ask",
    "description": "Name of the permission level where every action that changes something waits for your approval. Chat panel permission menu and button, Settings > Agents > Permission level. Keep it short: the button beside Send shows it."
  },
  "agent.permission.auto": {
    "message": "Auto",
    "description": "Name of the permission level where the agent acts alone on safe, reversible actions and asks before anything others see. Chat panel permission menu and button, Settings > Agents > Permission level. Keep it short."
  },
  "agent.permission.full": {
    "message": "Full permission",
    "description": "Name of the permission level where nothing waits for your approval. Chat panel permission menu, Settings > Agents > Permission level."
  },
  "agent.permission.fullShort": {
    "message": "Full",
    "description": "Short name of the Full permission level, on the small button beside Send in the chat panel (refreshPickers in agent/panel.js)."
  },
  "agent.permission.askHint": {
    "message": "Asks before every action",
    "description": "Chat panel permission menu: second line under Ask. One short line; the menu is narrow."
  },
  "agent.permission.autoHint": {
    "message": "Asks before deleting or what others see",
    "description": "Chat panel permission menu: second line under Auto, e.g. it still asks before deleting mail, invitations and changes in other systems. One short line; the menu is narrow."
  },
  "agent.permission.fullHint": {
    "message": "Never asks, not even before sending",
    "description": "Chat panel permission menu: second line under Full permission. Sending covers messages and calendar invitations. One short line; the menu is narrow."
  },
  "agent.permission.onlyInSettings": {
    "message": "For {name}, only in Settings",
    "description": "Chat panel permission menu: second line under a level that can't be picked, in a chat of Hermes or OpenClaw, for a level looser than the one in Settings > Agents. That agent's chats can be stricter than Settings but not looser; the user can pick the level for all chats in Settings. name is the agent's name, e.g. \"Hermes\". One short line; the menu is narrow."
  },
  "agent.panel.width": {
    "message": "Chat panel width",
    "description": "Chat panel: accessible name of the divider that resizes the panel."
  },
  "agent.plan.done": {
    "message": "Done",
    "description": "Chat transcript plan rows: status of a finished step."
  },
  "agent.plan.failed": {
    "message": "Failed",
    "description": "Chat transcript plan rows: status of a step that did not work."
  },
  "agent.plan.proposed": {
    "message": "Proposed",
    "description": "Chat transcript plan rows: status of a proposed step."
  },
  "agent.plan.running": {
    "message": "In progress",
    "description": "Chat transcript plan rows: status of a step the agent is working on."
  },
  "agent.plan.skipped": {
    "message": "Skipped",
    "description": "Chat transcript plan rows: status of a step the agent skipped."
  },
  "agent.plan.todo": {
    "message": "To do",
    "description": "Chat transcript plan rows: status of a step that is planned."
  },
  "agent.setup.disabled": {
    "message": "{name} is turned off.",
    "description": "Chat panel, above the disabled input: the agent is switched off in Settings."
  },
  "agent.setup.missing": {
    "message": "{name} isn't installed on this computer.",
    "description": "Chat panel, above the disabled input: the agent program was not found."
  },
  "agent.setup.unauthorized": {
    "message": "{name} didn't accept the API key.",
    "description": "Chat panel, above the disabled input: the agent rejected the key."
  },
  "agent.setup.unconfigured": {
    "message": "{name} isn't set up yet.",
    "description": "Chat panel, above the disabled input: the agent needs an address or key; name is the agent."
  },
  "agent.status.disabled": {
    "message": "Turned off",
    "description": "Chat panel and Settings: agent status when it is switched off in Settings."
  },
  "agent.status.missing": {
    "message": "Not installed",
    "description": "Chat panel and Settings: agent status when its program is not found on this computer."
  },
  "agent.status.offline": {
    "message": "Can't be reached",
    "description": "Chat panel and Settings: agent status when its server or program does not answer."
  },
  "agent.status.ready": {
    "message": "Ready",
    "description": "Chat panel and Settings: agent status when it can take a message."
  },
  "agent.status.starting": {
    "message": "Starting...",
    "description": "Chat panel and Settings: agent status while Rukoo connects to the agents."
  },
  "agent.status.unauthorized": {
    "message": "Key not accepted",
    "description": "Chat panel and Settings: agent status when the agent rejects the API key or token."
  },
  "agent.status.unavailable": {
    "message": "Agents aren't available",
    "description": "Chat panel header and input: shown when this build has no agent support."
  },
  "agent.status.unconfigured": {
    "message": "Not set up",
    "description": "Chat panel and Settings: agent status when the address or key is missing."
  },
  "agent.status.unknown": {
    "message": "Status unknown",
    "description": "Chat panel and Settings: agent status when Rukoo cannot tell."
  },
  "agent.tools.generic.command": {
    "message": "Ran a command",
    "description": "Chat transcript tool chip: the agent ran a shell or terminal command."
  },
  "agent.tools.generic.code": {
    "message": "Ran code",
    "description": "Chat transcript tool chip: the agent ran a piece of code."
  },
  "agent.tools.generic.webSearch": {
    "message": "Searched the web",
    "description": "Chat transcript tool chip: the agent ran a web search."
  },
  "agent.tools.generic.webPage": {
    "message": "Opened a web page",
    "description": "Chat transcript tool chip: the agent fetched or read a web page."
  },
  "agent.tools.generic.browser": {
    "message": "Used the browser",
    "description": "Chat transcript tool chip: the agent clicked or navigated in a browser."
  },
  "agent.tools.generic.readFile": {
    "message": "Read a file",
    "description": "Chat transcript tool chip: the agent read a file on its own machine."
  },
  "agent.tools.generic.changeFiles": {
    "message": "Changed files",
    "description": "Chat transcript tool chip: the agent wrote or edited files."
  },
  "agent.tools.generic.subagent": {
    "message": "Ran a subagent",
    "description": "Chat transcript tool chip: the agent handed part of the work to a helper agent."
  },
  "agent.tools.generic.findFiles": {
    "message": "Looked through files",
    "description": "Chat transcript tool chip: the agent searched or listed files."
  },
  "agent.tools.generic.todo": {
    "message": "Updated its to-do list",
    "description": "Chat transcript tool chip: the agent updated its own internal to-do list."
  },
  "agent.tools.generic.lookupTools": {
    "message": "Looked up its tools",
    "description": "Chat transcript tool chip: the agent looked up which tools it has."
  },
  "agent.tools.generic.memory": {
    "message": "Checked its memory",
    "description": "Chat transcript tool chip: the agent read or searched its long-term memory."
  },
  "agent.tools.generic.skill": {
    "message": "Read a skill",
    "description": "Chat transcript tool chip: the agent read one of its skills (instructions it keeps)."
  },
  "agent.tools.generic.pastChats": {
    "message": "Searched past chats",
    "description": "Chat transcript tool chip: the agent searched its earlier conversations."
  },
  "agent.tools.generic.schedule": {
    "message": "Scheduled a job",
    "description": "Chat transcript tool chip: the agent created or changed a scheduled job."
  },
  "agent.tools.generic.sendMessage": {
    "message": "Sent a message",
    "description": "Chat transcript tool chip: the agent sent a message on another channel such as WhatsApp or Slack."
  },
  "agent.tools.generic.image": {
    "message": "Made an image",
    "description": "Chat transcript tool chip: the agent generated an image."
  },
  "agent.tools.generic.vision": {
    "message": "Looked at an image",
    "description": "Chat transcript tool chip: the agent analysed an image."
  },
  "agent.tools.get_context": {
    "message": "Looked at your screen",
    "description": "Chat transcript tool chip: the agent read what Rukoo shows (open email, selection, draft)."
  },
  "agent.tools.get_draft": {
    "message": "Checked the draft",
    "description": "Chat transcript tool chip: the agent read the composer."
  },
  "agent.tools.mail_action": {
    "message": "Proposed a mail action",
    "description": "Chat transcript tool chip: the agent asked Rukoo to archive, move or mark mail."
  },
  "agent.tools.propose_action": {
    "message": "Asked for approval",
    "description": "Chat transcript tool chip: the agent proposed an action and waits for you."
  },
  "agent.tools.read_attachment": {
    "message": "Read an attachment",
    "description": "Chat transcript tool chip: the agent opened an attachment."
  },
  "agent.tools.read_chat_file": {
    "message": "Read an attached file",
    "description": "Chat transcript tool chip: the agent opened a file you attached to a message (read_chat_file)."
  },
  "agent.tools.read_skill": {
    "message": "Read a skill",
    "description": "Chat transcript tool chip (panel.js toolProps): the agent read one of Rukoo's skills with read_skill."
  },
  "agent.tools.read_message": {
    "message": "Read an email",
    "description": "Chat transcript tool chip: the agent read a full email."
  },
  "agent.tools.running.read_chat_file": {
    "message": "Reading an attached file...",
    "description": "Chat transcript tool chip while it runs: the agent reads a file you attached. Present tense; the done form is agent.tools.read_chat_file."
  },
  "agent.tools.search_mail": {
    "message": "Searched your mail",
    "description": "Chat transcript tool chip: the agent searched your mailboxes."
  },
  "agent.tools.show_plan": {
    "message": "Made a plan",
    "description": "Chat transcript tool chip: the agent showed a plan of steps."
  },
  "agent.tools.show_sources": {
    "message": "Gathered sources",
    "description": "Chat transcript tool chip: the agent showed the sources it used."
  },
  "agent.tools.unknown": {
    "message": "Used a tool",
    "description": "Chat transcript tool chip: fallback when the tool has no name."
  },
  "agent.tools.write_draft": {
    "message": "Wrote the draft",
    "description": "Chat transcript tool chip: the agent wrote into the composer."
  },
  "agent.tools.running.get_context": {
    "message": "Looking at your screen...",
    "description": "Chat transcript tool chip while it runs: the agent reads what is on screen. Present tense; the done form is agent.tools.get_context."
  },
  "agent.tools.running.get_draft": {
    "message": "Checking the draft...",
    "description": "Chat transcript tool chip while it runs: the agent reads the composer. Present tense; the done form is agent.tools.get_draft."
  },
  "agent.tools.running.mail_action": {
    "message": "Proposing a mail action...",
    "description": "Chat transcript tool chip while it runs: the agent asks to archive, move or mark mail. Present tense; the done form is agent.tools.mail_action."
  },
  "agent.tools.running.propose_action": {
    "message": "Asking for approval...",
    "description": "Chat transcript tool chip while it runs: the agent proposes an action. Present tense; the done form is agent.tools.propose_action."
  },
  "agent.tools.running.read_attachment": {
    "message": "Reading an attachment...",
    "description": "Chat transcript tool chip while it runs: the agent reads an attachment. Present tense; the done form is agent.tools.read_attachment."
  },
  "agent.tools.running.read_skill": {
    "message": "Reading a skill...",
    "description": "Chat transcript tool chip while it runs (panel.js toolProps): the agent reads one of Rukoo's skills. Present tense; the done form is agent.tools.read_skill."
  },
  "agent.tools.running.read_message": {
    "message": "Reading an email...",
    "description": "Chat transcript tool chip while it runs: the agent reads an email. Present tense; the done form is agent.tools.read_message."
  },
  "agent.tools.running.search_mail": {
    "message": "Searching your mail...",
    "description": "Chat transcript tool chip while it runs: the agent searches your mail. Present tense; the done form is agent.tools.search_mail."
  },
  "agent.tools.running.show_plan": {
    "message": "Making a plan...",
    "description": "Chat transcript tool chip while it runs: the agent shows a plan. Present tense; the done form is agent.tools.show_plan."
  },
  "agent.tools.running.show_sources": {
    "message": "Gathering sources...",
    "description": "Chat transcript tool chip while it runs: the agent shows its sources. Present tense; the done form is agent.tools.show_sources."
  },
  "agent.tools.running.unknown": {
    "message": "Using a tool...",
    "description": "Chat transcript tool chip while it runs: a tool without a name. Present tense; the done form is agent.tools.unknown."
  },
  "agent.tools.running.write_draft": {
    "message": "Writing the draft...",
    "description": "Chat transcript tool chip while it runs: the agent writes into the composer. Present tense; the done form is agent.tools.write_draft."
  },
  "agent.tools.running.generic.command": {
    "message": "Running a command...",
    "description": "Chat transcript tool chip while the agent's own tool runs: a shell command. Present tense; the done form is agent.tools.generic.command."
  },
  "agent.tools.running.generic.code": {
    "message": "Running code...",
    "description": "Chat transcript tool chip while the agent's own tool runs: code execution. Present tense; the done form is agent.tools.generic.code."
  },
  "agent.tools.running.generic.webSearch": {
    "message": "Searching the web...",
    "description": "Chat transcript tool chip while the agent's own tool runs: a web search. Present tense; the done form is agent.tools.generic.webSearch."
  },
  "agent.tools.running.generic.webPage": {
    "message": "Opening a web page...",
    "description": "Chat transcript tool chip while the agent's own tool runs: fetching a web page. Present tense; the done form is agent.tools.generic.webPage."
  },
  "agent.tools.running.generic.browser": {
    "message": "Using the browser...",
    "description": "Chat transcript tool chip while the agent's own tool runs: browser automation. Present tense; the done form is agent.tools.generic.browser."
  },
  "agent.tools.running.generic.readFile": {
    "message": "Reading a file...",
    "description": "Chat transcript tool chip while the agent's own tool runs: reading a file. Present tense; the done form is agent.tools.generic.readFile."
  },
  "agent.tools.running.generic.changeFiles": {
    "message": "Changing files...",
    "description": "Chat transcript tool chip while the agent's own tool runs: writing or editing files. Present tense; the done form is agent.tools.generic.changeFiles."
  },
  "agent.tools.running.generic.subagent": {
    "message": "Running a subagent...",
    "description": "Chat transcript tool chip while the agent's own tool runs: a subagent. Present tense; the done form is agent.tools.generic.subagent."
  },
  "agent.tools.running.generic.findFiles": {
    "message": "Looking through files...",
    "description": "Chat transcript tool chip while the agent's own tool runs: a file search. Present tense; the done form is agent.tools.generic.findFiles."
  },
  "agent.tools.running.generic.todo": {
    "message": "Updating its to-do list...",
    "description": "Chat transcript tool chip while the agent's own tool runs: the agent's own to-do list. Present tense; the done form is agent.tools.generic.todo."
  },
  "agent.tools.running.generic.lookupTools": {
    "message": "Looking up its tools...",
    "description": "Chat transcript tool chip while the agent's own tool runs: the agent looks up which tools it has. Present tense; the done form is agent.tools.generic.lookupTools."
  },
  "agent.tools.running.generic.memory": {
    "message": "Checking its memory...",
    "description": "Chat transcript tool chip while the agent's own tool runs: the agent's memory. Present tense; the done form is agent.tools.generic.memory."
  },
  "agent.tools.running.generic.skill": {
    "message": "Reading a skill...",
    "description": "Chat transcript tool chip while the agent's own tool runs: the agent reads one of its skills. Present tense; the done form is agent.tools.generic.skill."
  },
  "agent.tools.running.generic.pastChats": {
    "message": "Searching past chats...",
    "description": "Chat transcript tool chip while the agent's own tool runs: the agent searches its past conversations. Present tense; the done form is agent.tools.generic.pastChats."
  },
  "agent.tools.running.generic.schedule": {
    "message": "Scheduling a job...",
    "description": "Chat transcript tool chip while the agent's own tool runs: a scheduled job. Present tense; the done form is agent.tools.generic.schedule."
  },
  "agent.tools.running.generic.sendMessage": {
    "message": "Sending a message...",
    "description": "Chat transcript tool chip while the agent's own tool runs: a message to another channel. Present tense; the done form is agent.tools.generic.sendMessage."
  },
  "agent.tools.running.generic.image": {
    "message": "Making an image...",
    "description": "Chat transcript tool chip while the agent's own tool runs: image generation. Present tense; the done form is agent.tools.generic.image."
  },
  "agent.tools.running.generic.vision": {
    "message": "Looking at an image...",
    "description": "Chat transcript tool chip while the agent's own tool runs: image analysis. Present tense; the done form is agent.tools.generic.vision."
  },
  "agent.tools.list_events": {
    "message": "Checked your calendar",
    "description": "Chat transcript tool chip: the agent read the events in your calendar (list_events). src/renderer/agent/panel.js (toolProps)."
  },
  "agent.tools.find_free_time": {
    "message": "Looked for free time",
    "description": "Chat transcript tool chip: the agent looked for open time in your calendars (find_free_time)."
  },
  "agent.tools.create_event": {
    "message": "Proposed an event",
    "description": "Chat transcript tool chip: the agent asked Rukoo to add an event to your calendar (create_event); Rukoo asks you first."
  },
  "agent.tools.get_booking_link": {
    "message": "Got your booking link",
    "description": "Chat transcript tool chip: the agent read the link to your booking page (get_booking_link)."
  },
  "agent.tools.running.list_events": {
    "message": "Checking your calendar...",
    "description": "Chat transcript tool chip while it runs: the agent reads your calendar. Present tense; the done form is agent.tools.list_events."
  },
  "agent.tools.running.find_free_time": {
    "message": "Looking for free time...",
    "description": "Chat transcript tool chip while it runs: the agent looks for open time in your calendars. Present tense; the done form is agent.tools.find_free_time."
  },
  "agent.tools.running.create_event": {
    "message": "Proposing an event...",
    "description": "Chat transcript tool chip while it runs: the agent asks Rukoo to add an event. Present tense; the done form is agent.tools.create_event."
  },
  "agent.tools.running.get_booking_link": {
    "message": "Getting your booking link...",
    "description": "Chat transcript tool chip while it runs: the agent reads the link to your booking page. Present tense; the done form is agent.tools.get_booking_link."
  },
  "agent.tools.read_meeting_transcript": {
    "message": "Read the meeting",
    "description": "Chat transcript tool chip: the agent read the transcript of a meeting Rukoo Voice recorded (read_meeting_transcript). src/renderer/agent/panel.js (toolProps)."
  },
  "agent.tools.search_meetings": {
    "message": "Searched your meetings",
    "description": "Chat transcript tool chip: the agent searched the transcripts of your recorded meetings, or listed them (search_meetings)."
  },
  "agent.tools.read_meeting_notes": {
    "message": "Read your meeting notes",
    "description": "Chat transcript tool chip: the agent read the notes you wrote about a meeting in Rukoo (read_meeting_notes)."
  },
  "agent.tools.get_meeting_frame": {
    "message": "Looked at the meeting's screen",
    "description": "Chat transcript tool chip: the agent looked at the screen Rukoo Voice recorded at a moment in a meeting (get_meeting_frame)."
  },
  "agent.tools.get_meeting_clip": {
    "message": "Looked at the meeting's screen over a stretch",
    "description": "Chat transcript tool chip: the agent looked at the frames, and perhaps a video clip, of the screen Rukoo Voice recorded between two moments of a meeting (get_meeting_clip)."
  },
  "agent.tools.get_meeting_media_link": {
    "message": "Got a link to the meeting's audio or screen",
    "description": "Chat transcript tool chip: the agent got a short-lived link to audio, a screen frame or screen video Rukoo Voice recorded in a meeting (get_meeting_media_link)."
  },
  "agent.tools.running.read_meeting_transcript": {
    "message": "Reading the meeting...",
    "description": "Chat transcript tool chip while it runs: the agent reads a meeting's transcript. Present tense; the done form is agent.tools.read_meeting_transcript."
  },
  "agent.tools.running.search_meetings": {
    "message": "Searching your meetings...",
    "description": "Chat transcript tool chip while it runs: the agent searches your recorded meetings. Present tense; the done form is agent.tools.search_meetings."
  },
  "agent.tools.running.read_meeting_notes": {
    "message": "Reading your meeting notes...",
    "description": "Chat transcript tool chip while it runs: the agent reads your notes about a meeting. Present tense; the done form is agent.tools.read_meeting_notes."
  },
  "agent.tools.running.get_meeting_frame": {
    "message": "Looking at the meeting's screen...",
    "description": "Chat transcript tool chip while it runs: the agent looks at the screen recorded in a meeting. Present tense; the done form is agent.tools.get_meeting_frame."
  },
  "agent.tools.running.get_meeting_clip": {
    "message": "Looking at the meeting's screen over a stretch...",
    "description": "Chat transcript tool chip while it runs: the agent looks at the screen recorded over a stretch of a meeting. Present tense; the done form is agent.tools.get_meeting_clip."
  },
  "agent.tools.running.get_meeting_media_link": {
    "message": "Getting a link to the meeting's audio or screen...",
    "description": "Chat transcript tool chip while it runs: the agent asks for a link to a meeting's audio or screen. Present tense; the done form is agent.tools.get_meeting_media_link."
  },
  "agent.approval.calendarTitle": {
    "message": "{name} wants to add an event to your calendar",
    "description": "Chat transcript approval card title when an agent asks to add an event (create_event). name: agent name. src/renderer/agent/panel.js (localizeEvent)."
  },
  "agent.approval.calendarDecided": {
    "message": "Add \"{title}\" to your calendar",
    "description": "Chat transcript calendar approval card once you answered it: its title, next to Approved or Declined. title: the event's title."
  },
  "agent.approval.calendarFieldTitle": {
    "message": "Title",
    "description": "Chat transcript calendar approval card: label of the event's title."
  },
  "agent.approval.calendarFieldWhen": {
    "message": "When",
    "description": "Chat transcript calendar approval card: label of the event's date and times."
  },
  "agent.approval.calendarFieldCalendar": {
    "message": "Calendar",
    "description": "Chat transcript calendar approval card: label of the calendar the event goes in, shown as the calendar's name and its account."
  },
  "agent.approval.calendarFieldGuests": {
    "message": "Guests",
    "description": "Chat transcript calendar approval card: label of the people who get an invitation."
  },
  "agent.approval.calendarFieldVideo": {
    "message": "Video call",
    "description": "Chat transcript calendar approval card: label before \"Google Meet\" when the event gets a Meet link."
  },
  "agent.approval.calendarFieldLocation": {
    "message": "Location",
    "description": "Chat transcript calendar approval card: label of the event's place."
  },
  "agent.approval.calendarFieldDescription": {
    "message": "Description",
    "description": "Chat transcript calendar approval card: label of the event's notes, which guests see."
  },
  "agent.approval.calendarAllDay": {
    "message": "{days}, all day",
    "description": "Chat transcript calendar approval card: when an all-day event takes place. days: a date or a range of dates, such as \"Tue, 3 Nov 2026\"."
  },
  "agent.approval.calendarInvites": {
    "message": "Guests get an invitation from {address}.",
    "description": "Chat transcript calendar approval card: line under the title when the event has guests. address: the email account whose calendar sends the invitations."
  },
  "agent.approval.calendarCreate": {
    "message": "Create",
    "description": "Chat transcript calendar approval card: button that adds the event to the calendar."
  },
  "agent.calendar.created": {
    "message": "Added \"{title}\" to your calendar",
    "description": "Chat transcript notice after you approved an event an agent asked for and Rukoo added it. title: the event's title."
  },
  "agent.calendar.createdInvited": {
    "message": { "one": "Added \"{title}\" to your calendar and invited {count} guest", "other": "Added \"{title}\" to your calendar and invited {count} guests" },
    "description": "Chat transcript notice after Rukoo added an event an agent asked for, with guests who got an invitation from your calendar. title: the event's title; count: how many guests."
  },
  "agent.calendar.failed": {
    "message": "Couldn't add \"{title}\" to your calendar",
    "description": "Chat transcript notice when Rukoo couldn't add an event you approved; the calendar's error is shown under it. title: the event's title."
  },
  "calendar.title": {
    "message": "Calendar",
    "description": "Calendar view: accessible name of the section that replaces the message list and reader. src/renderer/app.js (renderShell)"
  },
  "calendar.mode.label": {
    "message": "Mail, calendar or meetings",
    "description": "Sidebar: accessible name of the switch at the top that chooses between the mailbox, the calendar and the meetings. src/renderer/app.js (renderModeSwitch)"
  },
  "calendar.mode.mail": {
    "message": "Mail",
    "description": "Sidebar switch at the top: the segment that shows the mailbox. Short; it shares a narrow sidebar with the Calendar segment. src/renderer/app.js (renderModeSwitch)"
  },
  "calendar.mode.mailShortcut": {
    "message": "Mail (Ctrl+1)",
    "description": "Tooltip of the Mail segment in the sidebar switch, with its keyboard shortcut. src/renderer/app.js (renderModeSwitch)"
  },
  "calendar.mode.calendar": {
    "message": "Calendar",
    "description": "Sidebar switch at the top: the segment that shows the calendar. Short; it shares a narrow sidebar with the Mail segment. src/renderer/app.js (renderModeSwitch)"
  },
  "calendar.mode.calendarShortcut": {
    "message": "Calendar (Ctrl+2)",
    "description": "Tooltip of the Calendar segment in the sidebar switch, with its keyboard shortcut. src/renderer/app.js (renderModeSwitch)"
  },
  "calendar.sidebar.calendars": {
    "message": "Calendars",
    "description": "Calendar sidebar: accessible name of the list of calendars, grouped by account, that can be shown or hidden. src/renderer/calendar.js (sidebarHtml)"
  },
  "calendar.nav.today": {
    "message": "Today",
    "description": "Calendar toolbar button that goes back to the current day or week. src/renderer/calendar.js (headHtml)"
  },
  "calendar.nav.todayShortcut": {
    "message": "Today (T)",
    "description": "Tooltip of the calendar's Today button, with its keyboard shortcut. src/renderer/calendar.js (headHtml)"
  },
  "calendar.nav.prev.day": {
    "message": "Previous day (←)",
    "description": "Tooltip of the calendar toolbar's back arrow in the day view, with its keyboard shortcut. src/renderer/calendar.js (headHtml)"
  },
  "calendar.nav.prev.week": {
    "message": "Previous week (←)",
    "description": "Tooltip of the calendar toolbar's back arrow in the week view, with its keyboard shortcut. src/renderer/calendar.js (headHtml)"
  },
  "calendar.nav.next.day": {
    "message": "Next day (→)",
    "description": "Tooltip of the calendar toolbar's forward arrow in the day view, with its keyboard shortcut. src/renderer/calendar.js (headHtml)"
  },
  "calendar.nav.next.week": {
    "message": "Next week (→)",
    "description": "Tooltip of the calendar toolbar's forward arrow in the week view, with its keyboard shortcut. src/renderer/calendar.js (headHtml)"
  },
  "calendar.views.label": {
    "message": "Calendar view",
    "description": "Accessible name of the switch in the calendar toolbar that chooses between Day, Week, Month and Upcoming. src/renderer/calendar.js (viewSwitch)"
  },
  "calendar.views.day": {
    "message": "Day",
    "description": "Calendar toolbar switch: one day at a time. src/renderer/calendar.js (viewSwitch)"
  },
  "calendar.views.dayShortcut": {
    "message": "Day (D)",
    "description": "Tooltip of the Day segment, with its keyboard shortcut. src/renderer/calendar.js (viewSwitch)"
  },
  "calendar.views.week": {
    "message": "Week",
    "description": "Calendar toolbar switch: a week at a time, the default. src/renderer/calendar.js (viewSwitch)"
  },
  "calendar.views.weekShortcut": {
    "message": "Week (W)",
    "description": "Tooltip of the Week segment, with its keyboard shortcut. src/renderer/calendar.js (viewSwitch)"
  },
  "calendar.views.upcoming": {
    "message": "Upcoming",
    "description": "Calendar toolbar switch and title: a list of the events in the coming weeks, grouped by day. src/renderer/calendar.js (viewSwitch, headHtml)"
  },
  "calendar.views.upcomingShortcut": {
    "message": "Upcoming (U)",
    "description": "Tooltip of the Upcoming segment, with its keyboard shortcut. src/renderer/calendar.js (viewSwitch)"
  },
  "calendar.views.more": {
    "message": "Show more",
    "description": "Button under the Upcoming list that adds the next 30 days. src/renderer/calendar.js (listHtml)"
  },
  "calendar.sync.shortcut": {
    "message": "Sync calendars (F5)",
    "description": "Tooltip of the sync button in the calendar toolbar, with its keyboard shortcut. src/renderer/calendar.js (render)"
  },
  "calendar.dates.tomorrow": {
    "message": "Tomorrow",
    "description": "Day heading in the Upcoming list for the day after today. src/renderer/calendar.js (listHtml)"
  },
  "calendar.empty.title": {
    "message": "No calendars yet",
    "description": "Calendar view when none of the accounts has a calendar Rukoo can show. src/renderer/calendar.js (render)"
  },
  "calendar.empty.help": {
    "message": "Rukoo shows the calendars of Google accounts, and of other accounts with a CalDAV server in their Server settings. Add an account under Settings → Accounts.",
    "description": "Calendar view, under calendar.empty.title: which accounts have a calendar and where to add one. src/renderer/calendar.js (render)"
  },
  "calendar.empty.upcoming": {
    "message": { "one": "Nothing planned in the next {count} day", "other": "Nothing planned in the next {count} days" },
    "description": "Upcoming list when no shown calendar has an event in the period; count is the number of days. src/renderer/calendar.js (listHtml)"
  },
  "calendar.connect.title": {
    "message": "Show the calendar of {email}",
    "description": "Notice above the calendar for a Google account that hasn't given Rukoo access to its calendar; email is the account's address. src/renderer/calendar.js (noticesHtml)"
  },
  "calendar.connect.help": {
    "message": "Sign in to Google again and allow access to the calendar. Access to your mail stays as it is.",
    "description": "Notice above the calendar, under calendar.connect.title: what reconnecting does. src/renderer/calendar.js (noticesHtml)"
  },
  "calendar.connect.action": {
    "message": "Connect calendar",
    "description": "Button in the calendar notice and row in the calendar sidebar that starts the Google sign-in for calendar access. src/renderer/calendar.js (noticesHtml, sidebarHtml)"
  },
  "calendar.connect.later": {
    "message": "Not now",
    "description": "Button in the calendar notice that hides it for this account; the sidebar still offers calendar.connect.action. src/renderer/calendar.js (noticesHtml)"
  },
  "calendar.errors.openCloud": {
    "message": "Open Google Cloud",
    "description": "Button in the calendar notice when the Google Calendar API is turned off for the user's OAuth client; it opens the page that turns it on. src/renderer/calendar.js (noticesHtml)"
  },
  "calendar.errors.noWritable": {
    "message": "None of your calendars takes new events.",
    "description": "Toast when the user wants to create an event but every shown calendar is read-only. src/renderer/calendar.js (eventDialog)"
  },
  "calendar.errors.serverSettings": {
    "message": "Server settings",
    "description": "Button in the calendar notice of an account whose CalDAV server failed; it opens that account's Server settings. src/renderer/calendar.js (noticesHtml)"
  },
  "calendar.event.new": {
    "message": "New event",
    "description": "Calendar sidebar button that opens the dialog for a new event. src/renderer/calendar.js (sidebarHtml)"
  },
  "calendar.event.newShortcut": {
    "message": "New event (Ctrl+N)",
    "description": "Tooltip of the New event button, with its keyboard shortcut. src/renderer/calendar.js (sidebarHtml)"
  },
  "calendar.event.noTitle": {
    "message": "(No title)",
    "description": "Calendar: shown for an event without a title. src/renderer/calendar.js (titleOf)"
  },
  "calendar.event.allDay": {
    "message": "All day",
    "description": "Calendar: label of the all-day row, the all-day checkbox in the event dialog, and the time of an all-day event in the Upcoming list. src/renderer/calendar.js"
  },
  "calendar.event.from": {
    "message": "from {time}",
    "description": "Upcoming list: time of an event that starts on this day and ends on a later one; time is the start time. src/renderer/calendar.js (rowHtml)"
  },
  "calendar.event.until": {
    "message": "until {time}",
    "description": "Upcoming list: time of an event that started on an earlier day and ends on this one; time is the end time. src/renderer/calendar.js (rowHtml)"
  },
  "calendar.event.invitation": {
    "message": "Invitation",
    "description": "Upcoming list: pill on an invitation the user hasn't answered yet. src/renderer/calendar.js (rowHtml)"
  },
  "calendar.event.series": {
    "message": "Repeats",
    "description": "Event card: row for an event that is one of a recurring series. src/renderer/calendar.js (popHtml)"
  },
  "calendar.event.join": {
    "message": "Join video call",
    "description": "Event card: link to the event's video call, such as Google Meet. src/renderer/calendar.js (popHtml)"
  },
  "calendar.event.now": {
    "message": "Now",
    "description": "Upcoming list: under the time of an event that is happening at this moment. src/renderer/calendar.js (rowHtml)"
  },
  "calendar.event.videoCall": {
    "message": "Video call",
    "description": "Upcoming list: marks an event that has a video call link, such as Google Meet. src/renderer/calendar.js (rowHtml)"
  },
  "calendar.event.openGoogle": {
    "message": "Open in Google Calendar",
    "description": "Tooltip of the event card button that opens the event in Google Calendar in the browser. src/renderer/calendar.js (popHtml)"
  },
  "calendar.event.editShortcut": {
    "message": "Edit (E)",
    "description": "Tooltip of the event card's edit button, with its keyboard shortcut. src/renderer/calendar.js (popHtml)"
  },
  "calendar.event.deleteShortcut": {
    "message": "Delete (Delete)",
    "description": "Tooltip of the event card's delete button, with its keyboard shortcut. src/renderer/calendar.js (popHtml)"
  },
  "calendar.event.deleteTitle": {
    "message": "Delete this event?",
    "description": "Confirmation dialog title before an event is deleted. src/renderer/calendar.js (removeEvent)"
  },
  "calendar.event.deleteHelp": {
    "message": "\"{title}\" will be removed from the calendar.",
    "description": "Confirmation dialog text before an event is deleted; title is the event's title. src/renderer/calendar.js (removeEvent)"
  },
  "calendar.event.created": {
    "message": "Event created",
    "description": "Toast after a new event was saved in the calendar. src/renderer/calendar.js (eventDialog)"
  },
  "calendar.event.deleted": {
    "message": "Event deleted",
    "description": "Toast after an event was deleted. src/renderer/calendar.js (removeEvent)"
  },
  "calendar.guests.count": {
    "message": { "one": "{count} guest", "other": "{count} guests" },
    "description": "Event card: heading of the guest list; count includes the organizer and the user. src/renderer/calendar.js (popHtml)"
  },
  "calendar.guests.more": {
    "message": { "one": "and {count} more", "other": "and {count} more" },
    "description": "Event card: last line of a shortened guest list; count is the number of guests not shown. src/renderer/calendar.js (popHtml)"
  },
  "calendar.guests.organizer": {
    "message": "Organizer",
    "description": "Event card: label after the name of the guest who organizes the event. src/renderer/calendar.js (popHtml)"
  },
  "calendar.guests.organizedBy": {
    "message": "Organized by {name}",
    "description": "Event card: who organizes an event without a guest list; name is a person's name or address. src/renderer/calendar.js (popHtml)"
  },
  "calendar.response.needsAction": {
    "message": "No answer yet",
    "description": "Event card guest list and accessible event names: a guest, or the user, hasn't answered the invitation. src/renderer/calendar.js"
  },
  "calendar.response.accepted": {
    "message": "Going",
    "description": "Event card guest list: the guest accepted the invitation. src/renderer/calendar.js"
  },
  "calendar.response.tentative": {
    "message": "Maybe",
    "description": "Event card guest list and accessible event names: the guest, or the user, answered maybe. src/renderer/calendar.js"
  },
  "calendar.response.declined": {
    "message": "Not going",
    "description": "Event card guest list and accessible event names: the guest, or the user, declined. src/renderer/calendar.js"
  },
  "calendar.rsvp.question": {
    "message": "Going?",
    "description": "Event card: question before the three answers to an invitation. src/renderer/calendar.js (popHtml)"
  },
  "calendar.rsvp.accepted": {
    "message": "Yes",
    "description": "Event card: answer that accepts the invitation. src/renderer/calendar.js (popHtml)"
  },
  "calendar.rsvp.tentative": {
    "message": "Maybe",
    "description": "Event card: answer that accepts the invitation tentatively. src/renderer/calendar.js (popHtml)"
  },
  "calendar.rsvp.declined": {
    "message": "No",
    "description": "Event card: answer that declines the invitation. src/renderer/calendar.js (popHtml)"
  },
  "calendar.rsvp.sent.accepted": {
    "message": "Invitation accepted",
    "description": "Toast after the user accepted an invitation; Google tells the organizer. src/renderer/calendar.js (respond)"
  },
  "calendar.rsvp.sent.tentative": {
    "message": "Answered maybe",
    "description": "Toast after the user answered maybe to an invitation; Google tells the organizer. src/renderer/calendar.js (respond)"
  },
  "calendar.rsvp.sent.declined": {
    "message": "Invitation declined",
    "description": "Toast after the user declined an invitation; Google tells the organizer. src/renderer/calendar.js (respond)"
  },
  "calendar.series.one": {
    "message": "This event",
    "description": "Choice for a recurring event: only the occurrence the user opened. src/renderer/calendar.js (seriesChoice)"
  },
  "calendar.series.all": {
    "message": "All events in the series",
    "description": "Choice for a recurring event: every occurrence of the series. src/renderer/calendar.js (seriesChoice)"
  },
  "calendar.series.deleteTitle": {
    "message": "Delete a recurring event",
    "description": "Title of the choice between deleting one occurrence or the whole series. src/renderer/calendar.js (removeEvent)"
  },
  "calendar.series.respondTitle": {
    "message": "Answer a recurring event",
    "description": "Title of the choice between answering one occurrence or the whole series. src/renderer/calendar.js (respond)"
  },
  "calendar.notify.updateTitle": {
    "message": "Send the change to the guests?",
    "description": "Dialog after the user changed an event they organize that has guests. src/renderer/calendar.js (notifyGuests)"
  },
  "calendar.notify.deleteTitle": {
    "message": "Tell the guests the event is cancelled?",
    "description": "Dialog before deleting an event the user organizes that has guests. src/renderer/calendar.js (notifyGuests)"
  },
  "calendar.notify.help": {
    "message": { "one": "Google emails {count} guest about it.", "other": "Google emails {count} guests about it." },
    "description": "Dialog text under calendar.notify.*Title; count is the number of guests besides the user. src/renderer/calendar.js (notifyGuests)"
  },
  "calendar.notify.send": {
    "message": "Send",
    "description": "Dialog button: save the change and let Google email the guests. src/renderer/calendar.js (notifyGuests)"
  },
  "calendar.notify.dontSend": {
    "message": "Don't send",
    "description": "Dialog button: save the change without emailing the guests. src/renderer/calendar.js (notifyGuests)"
  },
  "calendar.dialog.newTitle": {
    "message": "New event",
    "description": "Title of the dialog that creates an event. src/renderer/calendar.js (eventDialog)"
  },
  "calendar.dialog.editTitle": {
    "message": "Edit event",
    "description": "Title of the dialog that changes an event. src/renderer/calendar.js (eventDialog)"
  },
  "calendar.dialog.title": {
    "message": "Title",
    "description": "Event dialog: accessible name of the title field. src/renderer/calendar.js (formHtml)"
  },
  "calendar.dialog.titlePlaceholder": {
    "message": "Add a title",
    "description": "Event dialog: placeholder of the title field. src/renderer/calendar.js (formHtml)"
  },
  "calendar.dialog.starts": {
    "message": "Starts",
    "description": "Event dialog: label of the start date and time. src/renderer/calendar.js (formHtml)"
  },
  "calendar.dialog.ends": {
    "message": "Ends",
    "description": "Event dialog: label of the end date and time; for an all-day event the last day. src/renderer/calendar.js (formHtml)"
  },
  "calendar.dialog.calendar": {
    "message": "Calendar",
    "description": "Event dialog: label of the calendar the event goes in, grouped by account. src/renderer/calendar.js (formHtml)"
  },
  "calendar.dialog.location": {
    "message": "Location",
    "description": "Event dialog: label of the location field. src/renderer/calendar.js (formHtml)"
  },
  "calendar.dialog.description": {
    "message": "Description",
    "description": "Event dialog: label of the description field. src/renderer/calendar.js (formHtml)"
  },
  "calendar.dialog.seriesNote": {
    "message": "This event repeats. When you save, you choose which events change.",
    "description": "Event dialog: note when the user edits one occurrence of a recurring event; Save then asks this event, this and following, or all. src/renderer/calendar-event.js (formHtml)"
  },
  "calendar.dialog.missingTime": {
    "message": "Fill in when the event starts and ends.",
    "description": "Event dialog error when a date or time field is empty. src/renderer/calendar.js (readForm)"
  },
  "calendar.dialog.noCalendar": {
    "message": "Choose a calendar.",
    "description": "Event dialog error when no calendar is chosen. src/renderer/calendar.js (readForm)"
  },
  "calendar.dialog.endBeforeStart": {
    "message": "The event ends before it starts.",
    "description": "Event dialog error when the end is before or at the start. src/renderer/calendar.js (readForm)"
  },
  "calendar.dialog.changedElsewhere": {
    "message": "This event was changed elsewhere while you edited it. The form now shows the latest version with your changes; save again to keep them.",
    "description": "Event dialog notice after Save failed because the event changed in Google Calendar (or another app) after the dialog opened. The fields now hold the latest version with the user's own changes on top. src/renderer/calendar.js (eventDialog)"
  },
  "calendar.views.month": {
    "message": "Month",
    "description": "Calendar toolbar switch: a month at a time, as a grid of weeks. src/renderer/calendar.js (viewSwitch)"
  },
  "calendar.views.monthShortcut": {
    "message": "Month (M)",
    "description": "Tooltip of the Month segment, with its keyboard shortcut. src/renderer/calendar.js (viewSwitch)"
  },
  "calendar.nav.prev.month": {
    "message": "Previous month (←)",
    "description": "Tooltip of the calendar toolbar's back arrow in the month view, with its keyboard shortcut. src/renderer/calendar.js (headHtml)"
  },
  "calendar.nav.next.month": {
    "message": "Next month (→)",
    "description": "Tooltip of the calendar toolbar's forward arrow in the month view, with its keyboard shortcut. src/renderer/calendar.js (headHtml)"
  },
  "calendar.nav.panelShortcut": {
    "message": "{title} (A)",
    "description": "Tooltip of the calendar toolbar button that opens a side panel, with its keyboard shortcut; title is the panel's name, such as Availability. src/renderer/calendar.js (renderHead)"
  },
  "calendar.nav.notFound": {
    "message": "This event is no longer in your calendar.",
    "description": "Toast when a reminder, notification or search result points at an event that was deleted meanwhile. src/renderer/calendar.js (openEvent)"
  },
  "calendar.month.more": {
    "message": { "one": "+{count} more", "other": "+{count} more" },
    "description": "Month view: button at the bottom of a day that has more events than fit; count is how many are hidden. It opens that day. Keep it very short. src/renderer/calendar.js (monthWeekHtml)"
  },
  "calendar.month.moreLabel": {
    "message": { "one": "{count} more event on {date}", "other": "{count} more events on {date}" },
    "description": "Month view: accessible name of the +N more button; count is how many events are hidden, date the day, such as Tuesday 27 October. src/renderer/calendar.js (monthWeekHtml)"
  },
  "calendar.mini.prev": {
    "message": "Previous month",
    "description": "Tooltip of the back arrow of the small month calendar in the calendar sidebar and the Go to date dialog. src/renderer/calendar.js (miniHtml)"
  },
  "calendar.mini.next": {
    "message": "Next month",
    "description": "Tooltip of the forward arrow of the small month calendar in the calendar sidebar and the Go to date dialog. src/renderer/calendar.js (miniHtml)"
  },
  "calendar.goto.title": {
    "message": "Go to date",
    "description": "Title of the dialog that jumps the calendar to a chosen day; it opens with G or a click on the calendar's title. src/renderer/calendar.js (goToDate)"
  },
  "calendar.goto.shortcut": {
    "message": "Go to date (G)",
    "description": "Tooltip of the calendar's title in the toolbar, which opens the Go to date dialog, with its keyboard shortcut. src/renderer/calendar.js (headHtml)"
  },
  "calendar.goto.date": {
    "message": "Date",
    "description": "Label of the date field in the Go to date dialog. src/renderer/calendar.js (goToDate)"
  },
  "calendar.goto.show": {
    "message": "Show",
    "description": "Main button of the Go to date dialog: shows the chosen day in the calendar. src/renderer/calendar.js (goToDate)"
  },
  "calendar.goto.invalid": {
    "message": "Enter a date.",
    "description": "Error in the Go to date dialog when the date field is empty or incomplete. src/renderer/calendar.js (goToDate)"
  },
  "calendar.drag.moved": {
    "message": "Event moved",
    "description": "Toast after an event was dragged to another time or day; it offers Undo. src/renderer/calendar.js (saveChange)"
  },
  "calendar.drag.resized": {
    "message": "End time changed",
    "description": "Toast after an event's bottom edge was dragged to make it shorter or longer; it offers Undo. src/renderer/calendar.js (saveChange)"
  },
  "calendar.drag.undone": {
    "message": "Change undone",
    "description": "Toast after Undo put a dragged event back where it was. src/renderer/calendar.js (undoChange)"
  },
  "calendar.drag.seriesMove": {
    "message": "Move a recurring event",
    "description": "Title of the question which events of a series to move, after dragging one occurrence to another time. src/renderer/calendar.js (saveChange)"
  },
  "calendar.drag.seriesResize": {
    "message": "Change the length of a recurring event",
    "description": "Title of the question which events of a series to change, after dragging one occurrence's end. src/renderer/calendar.js (saveChange)"
  },
  "calendar.colors.heading": {
    "message": "Color",
    "description": "Calendar sidebar menu: accessible name of the row of color swatches for a calendar. src/renderer/calendar.js (calendarMenu)"
  },
  "calendar.colors.default": {
    "message": "Default color",
    "description": "Calendar sidebar menu: goes back to the calendar's own color (from Google or the server, else the account's) after one was picked. Shown with a dot in that color. src/renderer/calendar.js (calendarMenu)"
  },
  "calendar.colors.red": {
    "message": "Red",
    "description": "Name of a color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.colors.orange": {
    "message": "Orange",
    "description": "Name of a color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.colors.yellow": {
    "message": "Yellow",
    "description": "Name of a color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.colors.lime": {
    "message": "Lime",
    "description": "Name of a yellowish green color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.colors.green": {
    "message": "Green",
    "description": "Name of a color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.colors.turquoise": {
    "message": "Turquoise",
    "description": "Name of a color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.colors.lightBlue": {
    "message": "Light blue",
    "description": "Name of a color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.colors.blue": {
    "message": "Blue",
    "description": "Name of a color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.colors.indigo": {
    "message": "Indigo",
    "description": "Name of a bluish purple color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.colors.purple": {
    "message": "Purple",
    "description": "Name of a color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.colors.pink": {
    "message": "Pink",
    "description": "Name of a color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.colors.grey": {
    "message": "Gray",
    "description": "Name of a color swatch in a calendar's sidebar menu, read out and shown as a tooltip. src/renderer/calendar.js (PALETTE)"
  },
  "calendar.sidebar.options": {
    "message": "Options for {name}",
    "description": "Tooltip and accessible name of the ⋯ button on a calendar in the calendar sidebar, which opens its color and availability menu; name is the calendar's name. src/renderer/calendar.js (sidebarHtml)"
  },
  "calendar.sidebar.availability": {
    "message": "Counts toward availability",
    "description": "Calendar sidebar menu: check item that decides whether this calendar's busy times count when Rukoo works out when you are free (availability panel, booking page). Hiding a calendar doesn't change it. src/renderer/calendar.js (calendarMenu)"
  },
  "calendar.sidebar.availabilityShared": {
    "message": "Not your own calendar: when it's busy, you count as busy too.",
    "description": "Calendar sidebar menu: second line under 'Counts toward availability' for a calendar shared by someone else or subscribed to. It says what turning it on does: that calendar's busy times then make you unavailable on the availability panel and the booking page. src/renderer/calendar.js (calendarMenu)"
  },
  "calendar.search.open": {
    "message": "Search events (/)",
    "description": "Tooltip of the search button in the calendar toolbar, with its keyboard shortcut. src/renderer/calendar.js (render)"
  },
  "calendar.search.placeholder": {
    "message": "Search events",
    "description": "Placeholder and accessible name of the search field in the calendar toolbar. src/renderer/calendar.js (searchFieldHtml)"
  },
  "calendar.search.close": {
    "message": "Close search (Esc)",
    "description": "Tooltip of the button that closes the calendar search and shows the calendar again, with its keyboard shortcut. src/renderer/calendar.js (searchFieldHtml)"
  },
  "calendar.search.hint": {
    "message": "Search titles, places, descriptions and guests.",
    "description": "Calendar search, before anything is typed: what the search looks through. src/renderer/calendar.js (searchHtml)"
  },
  "calendar.search.searching": {
    "message": "Searching…",
    "description": "Calendar search while the first results are on their way. src/renderer/calendar.js (searchHtml)"
  },
  "calendar.search.results": {
    "message": { "one": "{count} event found", "other": "{count} events found" },
    "description": "Calendar search: line above the results; count is the number of events. src/renderer/calendar.js (searchHtml)"
  },
  "calendar.search.empty": {
    "message": "No events match “{query}”",
    "description": "Calendar search when nothing matches; query is what the user typed. src/renderer/calendar.js (searchHtml)"
  },
  "calendar.search.failed": {
    "message": "Couldn't search {email}.",
    "description": "Calendar search: one account's calendar couldn't be searched, so its events are missing; email is the account's address. src/renderer/calendar.js (searchHtml)"
  },
  "calendar.notices.remindersTitle": {
    "message": "Event reminders",
    "description": "Title of the one-time notice above the calendar after the update that added desktop reminders. src/renderer/calendar.js (upgradeNotice)"
  },
  "calendar.notices.remindersNew": {
    "message": "Rukoo now reminds you of events. Change this in Settings → Calendar.",
    "description": "Text of the one-time notice above the calendar after the update that added desktop reminders. src/renderer/calendar.js (upgradeNotice)"
  },
  "calendar.notices.openSettings": {
    "message": "Open settings",
    "description": "Button in the reminders notice above the calendar that opens Settings → Calendar. src/renderer/calendar.js (upgradeNotice)"
  },
  "calendar.notices.dismiss": {
    "message": "Got it",
    "description": "Button in the reminders notice above the calendar that hides it for good. src/renderer/calendar.js (upgradeNotice)"
  },
  "calendar.requests.label": {
    "message": "Request",
    "description": "Small label on a dashed block in the calendar for a booking request that waits for the user to accept or decline it. Keep it short. src/renderer/calendar.js (requestHtml, monthChipHtml, requestRowHtml)"
  },
  "calendar.requests.aria": {
    "message": "Booking request, {when}",
    "description": "Accessible name of a booking request in the calendar when the guest's name isn't known; when is its day and time. src/renderer/calendar.js (requestLabel)"
  },
  "calendar.requests.ariaNamed": {
    "message": "Booking request from {name}, {when}",
    "description": "Accessible name of a booking request in the calendar; name is the guest's name, when its day and time. src/renderer/calendar.js (requestLabel)"
  },
  "calendar.requests.booked": {
    "message": "Booked through your booking page",
    "description": "Tooltip of the small icon on an event that someone booked through the user's booking page; also read out with the event. src/renderer/calendar.js (bookingMark, ariaLabel)"
  },
  "calendar.mode.calendarRequests": {
    "message": { "one": "Calendar, {count} booking request", "other": "Calendar, {count} booking requests" },
    "description": "Accessible name of the Calendar segment in the sidebar switch while booking requests wait for an answer; count is how many. src/renderer/app.js (renderModeSwitch)"
  },
  "calendar.event.duplicate": {
    "message": "Duplicate",
    "description": "Tooltip of the event card button that opens the event dialog as a new event with the same fields. src/renderer/calendar-event.js (popHtml)"
  },
  "calendar.dialog.more": {
    "message": "More",
    "description": "Event dialog: the button that opens and closes the section with repeat, reminders, time zone, show as and visibility; a short summary of what is set follows it. src/renderer/calendar-event.js (formHtml)"
  },
  "calendar.quick.more": {
    "message": "More options",
    "description": "Button in the quick popover for a new event that opens the full event dialog with what was typed. src/renderer/calendar-event.js (quickHtml)"
  },
  "calendar.guests.label": {
    "message": "Guests",
    "description": "Event dialog: label of the guest field, where guests show as chips. src/renderer/calendar-event.js (formHtml)"
  },
  "calendar.guests.add": {
    "message": "Add guests",
    "description": "Placeholder and accessible name of the field that adds guests by address, with suggestions from synced mail. src/renderer/calendar-event.js (guestsHtml)"
  },
  "calendar.guests.optional": {
    "message": "Optional",
    "description": "Marks a guest whose attendance is optional: on the guest's chip, as the checkbox item in the chip's menu, and after the name on the event card. src/renderer/calendar-event.js (chipHtml, bindGuests, popHtml)"
  },
  "calendar.guests.remove": {
    "message": "Remove {name}",
    "description": "Accessible name of the small x button on a guest's chip; name is the guest's name or address. src/renderer/calendar-event.js (chipHtml)"
  },
  "calendar.guests.removeGuest": {
    "message": "Remove guest",
    "description": "Item in a guest chip's menu that takes the guest off the event. src/renderer/calendar-event.js (bindGuests)"
  },
  "calendar.guests.invalid": {
    "message": "\"{text}\" isn't an email address.",
    "description": "Line under the guest field when what was typed isn't an address; text is what was typed. src/renderer/calendar-event.js (bindGuests)"
  },
  "calendar.guests.fix": {
    "message": "Check the guest addresses.",
    "description": "Error in the event dialog or quick popover when Save was pressed while the guest field holds something that isn't an address. src/renderer/calendar-event.js (readForm, quickCreate)"
  },
  "calendar.meet.add": {
    "message": "Google Meet video call",
    "description": "Switch in the event dialog and the quick popover that adds Google Meet to the event, or removes it when switched off; only on Google calendars that allow Meet. src/renderer/calendar-event.js (formHtml, quickHtml)"
  },
  "calendar.conflict.one": {
    "message": "Overlaps {title} {time}",
    "description": "Conflict line in the event dialog and quick popover: another busy event overlaps the times; title is its title, time its times such as 14:00–15:00, or calendar.conflict.allDay. src/renderer/calendar-event.js (conflictWatcher)"
  },
  "calendar.conflict.many": {
    "message": { "one": "Overlaps {title} {time} and {count} more event", "other": "Overlaps {title} {time} and {count} more events" },
    "description": "Conflict line when several busy events overlap the times; title and time are the first one's, count is how many more there are. src/renderer/calendar-event.js (conflictWatcher)"
  },
  "calendar.conflict.allDay": {
    "message": "(all day)",
    "description": "Conflict line: takes the place of the times when the overlapping event lasts all day. src/renderer/calendar-event.js (conflictWatcher)"
  },
  "calendar.conflict.unchecked": {
    "message": "Couldn't check {email}.",
    "description": "Conflict line: one account's calendar couldn't be read, so its events may overlap without showing; email is that account's address. src/renderer/calendar-event.js (conflictWatcher)"
  },
  "calendar.conflict.uncheckedMany": {
    "message": { "one": "Couldn't check {count} account.", "other": "Couldn't check {count} accounts." },
    "description": "Conflict line: several accounts' calendars couldn't be read; count is how many. src/renderer/calendar-event.js (conflictWatcher)"
  },
  "calendar.move.note": {
    "message": "Saving moves the event to {calendar}.",
    "description": "Event dialog: line under the calendar field once another calendar of the same account is chosen for an existing event; calendar is its name. src/renderer/calendar-event.js (bindForm)"
  },
  "calendar.move.seriesNote": {
    "message": "Saving moves the whole series to {calendar}.",
    "description": "Event dialog: as calendar.move.note, for a recurring event, which moves as a whole; calendar is the name of the calendar chosen. src/renderer/calendar-event.js (bindForm)"
  },
  "calendar.move.done": {
    "message": "Moved to {calendar}",
    "description": "Toast after an event moved to another calendar of its account; calendar is that calendar's name. src/renderer/calendar-event.js (saveEdit)"
  },
  "calendar.notify.createTitle": {
    "message": "Send invitations to the guests?",
    "description": "Dialog before a new event with guests is saved on a Google calendar; followed by calendar.notify.help. src/renderer/calendar-event.js (askInvite)"
  },
  "calendar.series.following": {
    "message": "This and following events",
    "description": "Choice for a recurring event: the occurrence opened and every later one. Offered only to the organizer. src/renderer/calendar-event.js (seriesChoice)"
  },
  "calendar.series.editTitle": {
    "message": "Change a recurring event",
    "description": "Title of the choice of which occurrences a change in the event dialog is for. src/renderer/calendar-event.js (saveEdit)"
  },
  "calendar.recurrence.label": {
    "message": "Repeat",
    "description": "Event dialog, More section: label of the repeat choice. src/renderer/calendar-event.js (repeatHtml)"
  },
  "calendar.recurrence.loading": {
    "message": "Loading…",
    "description": "Event dialog: the repeat choice while the series' rule is being read. src/renderer/calendar-event.js (repeatHtml)"
  },
  "calendar.recurrence.none": {
    "message": "Does not repeat",
    "description": "Repeat choice: a single event. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.daily": {
    "message": "Every day",
    "description": "Repeat choice: daily. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.weekdays": {
    "message": "Every weekday (Monday to Friday)",
    "description": "Repeat choice: Monday to Friday every week. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.weekly": {
    "message": "Weekly on {weekday}",
    "description": "Repeat choice: every week on the start's weekday; weekday is its name, such as Tuesday. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.monthlyDay": {
    "message": "Monthly on day {day}",
    "description": "Repeat choice: every month on the start's day of the month; day is a number such as 27. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.monthlyFirst": {
    "message": "Monthly on the first {weekday}",
    "description": "Repeat choice: every month on the first such weekday, when the start is one; weekday is its name. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.monthlySecond": {
    "message": "Monthly on the second {weekday}",
    "description": "Repeat choice: every month on the second such weekday, when the start is one; weekday is its name. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.monthlyThird": {
    "message": "Monthly on the third {weekday}",
    "description": "Repeat choice: every month on the third such weekday, when the start is one; weekday is its name. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.monthlyFourth": {
    "message": "Monthly on the fourth {weekday}",
    "description": "Repeat choice: every month on the fourth such weekday, when the start is one; weekday is its name. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.monthlyLast": {
    "message": "Monthly on the last {weekday}",
    "description": "Repeat choice, offered when the start is in its month's last seven days: every month on the last such weekday; weekday is its name. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.yearly": {
    "message": "Yearly on {date}",
    "description": "Repeat choice: every year on the start's date; date is a day and month such as 27 October. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.custom": {
    "message": "Custom…",
    "description": "Repeat choice that opens the custom form: every N days, weeks, months or years, the weekdays, and when it ends. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.customSummary": {
    "message": "Custom repeat",
    "description": "Summary after the event dialog's More button when the event has a custom repeat. src/renderer/calendar-event.js (repeatSummary)"
  },
  "calendar.recurrence.kept": {
    "message": "Custom rule, kept",
    "description": "Repeat choice for a series whose rule, made in another app, is more than the editor can show; it is kept as it is unless another choice is made. src/renderer/calendar-event.js (repeatChoices)"
  },
  "calendar.recurrence.keptHelp": {
    "message": "Rukoo can't edit this rule. It stays as it is unless you choose another.",
    "description": "Line under the repeat choice when it shows calendar.recurrence.kept. src/renderer/calendar-event.js (repeatHtml)"
  },
  "calendar.recurrence.every": {
    "message": "Repeat every",
    "description": "Custom repeat form: text before the number field and the unit choice, as in 'Repeat every 2 weeks'. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.unitLabel": {
    "message": "Repeat unit",
    "description": "Custom repeat form: accessible name of the choice between days, weeks, months and years. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.unitDays": {
    "message": { "one": "day", "other": "days" },
    "description": "Custom repeat form: unit after the number in 'Repeat every 2 days'; count is that number. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.unitWeeks": {
    "message": { "one": "week", "other": "weeks" },
    "description": "Custom repeat form: unit after the number in 'Repeat every 2 weeks'; count is that number. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.unitMonths": {
    "message": { "one": "month", "other": "months" },
    "description": "Custom repeat form: unit after the number in 'Repeat every 2 months'; count is that number. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.unitYears": {
    "message": { "one": "year", "other": "years" },
    "description": "Custom repeat form: unit after the number in 'Repeat every 2 years'; count is that number. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.onDays": {
    "message": "Repeat on",
    "description": "Custom repeat form: accessible name of the row of weekday buttons for a weekly repeat. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.onMonthly": {
    "message": "Day of the month",
    "description": "Custom repeat form: accessible name of the choice which day of the month a monthly repeat falls on. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.onDay": {
    "message": "On day {day}",
    "description": "Custom repeat form, monthly: on the start's day of the month; day is a number. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.onFirst": {
    "message": "On the first {weekday}",
    "description": "Custom repeat form, monthly: on the first such weekday of the month; weekday is its name. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.onSecond": {
    "message": "On the second {weekday}",
    "description": "Custom repeat form, monthly: on the second such weekday of the month; weekday is its name. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.onThird": {
    "message": "On the third {weekday}",
    "description": "Custom repeat form, monthly: on the third such weekday of the month; weekday is its name. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.onFourth": {
    "message": "On the fourth {weekday}",
    "description": "Custom repeat form, monthly: on the fourth such weekday of the month; weekday is its name. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.onLast": {
    "message": "On the last {weekday}",
    "description": "Custom repeat form, monthly: on the last such weekday of the month; weekday is its name. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.ends": {
    "message": "Ends",
    "description": "Custom repeat form: label of the choice between never, on a date and after a number of times. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.never": {
    "message": "Never",
    "description": "Custom repeat form: the repeat doesn't end. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.on": {
    "message": "On",
    "description": "Custom repeat form: the repeat ends on the date in the field after it. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.untilLabel": {
    "message": "Last date",
    "description": "Custom repeat form: accessible name of the date the repeat ends on. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.after": {
    "message": "After",
    "description": "Custom repeat form: the repeat ends after the number of times in the field after it. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.countLabel": {
    "message": "Number of times",
    "description": "Custom repeat form: accessible name of the field with how many times the event happens. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.times": {
    "message": { "one": "time", "other": "times" },
    "description": "Custom repeat form: unit after the number field in 'After 10 times'; count is that number. src/renderer/calendar-event.js (customHtml)"
  },
  "calendar.recurrence.shortMonths": {
    "message": "Months with fewer than {day} days are skipped.",
    "description": "Warning under a monthly repeat on day 29, 30 or 31; day is that number. src/renderer/calendar-event.js (repeatHtml)"
  },
  "calendar.recurrence.badInterval": {
    "message": "Repeat every 1 to 99.",
    "description": "Event dialog error when the custom repeat's number is out of range. src/renderer/calendar-event.js (repeatProblem)"
  },
  "calendar.recurrence.noDays": {
    "message": "Choose at least one day.",
    "description": "Event dialog error when a custom weekly repeat has no weekday chosen. src/renderer/calendar-event.js (repeatProblem)"
  },
  "calendar.recurrence.badCount": {
    "message": "Repeat 1 to 999 times.",
    "description": "Event dialog error when the custom repeat's number of times is out of range. src/renderer/calendar-event.js (repeatProblem)"
  },
  "calendar.recurrence.badUntil": {
    "message": "The last date comes before the event.",
    "description": "Event dialog error when the custom repeat ends on a date before the event's start. src/renderer/calendar-event.js (repeatProblem)"
  },
  "calendar.reminders.label": {
    "message": "Reminders",
    "description": "Event dialog, More section: label of the event's reminders. src/renderer/calendar-event.js (remindersHtml)"
  },
  "calendar.reminders.add": {
    "message": "Add reminder",
    "description": "Event dialog: button that adds a reminder, up to five. src/renderer/calendar-event.js (remindersHtml)"
  },
  "calendar.reminders.remove": {
    "message": "Remove reminder",
    "description": "Event dialog: tooltip and accessible name of the x after a reminder. src/renderer/calendar-event.js (remindersHtml)"
  },
  "calendar.reminders.which": {
    "message": "Reminder {n}",
    "description": "Event dialog: accessible name of a reminder's choice; n is its number in the list. src/renderer/calendar-event.js (remindersHtml)"
  },
  "calendar.reminders.atStart": {
    "message": "At the start",
    "description": "Reminder choice for a timed event: when it starts. src/renderer/calendar-event.js (reminderLabel)"
  },
  "calendar.reminders.minutes": {
    "message": { "one": "{count} minute before", "other": "{count} minutes before" },
    "description": "Reminder choice for a timed event; count is the number of minutes before the start. src/renderer/calendar-event.js (reminderLabel)"
  },
  "calendar.reminders.hours": {
    "message": { "one": "{count} hour before", "other": "{count} hours before" },
    "description": "Reminder choice for a timed event; count is the number of hours before the start. src/renderer/calendar-event.js (reminderLabel)"
  },
  "calendar.reminders.days": {
    "message": { "one": "{count} day before", "other": "{count} days before" },
    "description": "Reminder choice for a timed event; count is the number of days before the start. src/renderer/calendar-event.js (reminderLabel)"
  },
  "calendar.reminders.weeks": {
    "message": { "one": "{count} week before", "other": "{count} weeks before" },
    "description": "Reminder choice for a timed event; count is the number of weeks before the start. src/renderer/calendar-event.js (reminderLabel)"
  },
  "calendar.reminders.before": {
    "message": "{time} before",
    "description": "Reminder set elsewhere to an uncommon time before a timed event; time is a length such as 45 min or 1 hr, 30 min. src/renderer/calendar-event.js (reminderLabel)"
  },
  "calendar.reminders.onTheDay": {
    "message": "On the day at {time}",
    "description": "Reminder choice for an all-day event: on its first day at a time such as 09:00. src/renderer/calendar-event.js (reminderLabel)"
  },
  "calendar.reminders.dayBefore": {
    "message": "The day before at {time}",
    "description": "Reminder choice for an all-day event: the day before at a time such as 09:00. src/renderer/calendar-event.js (reminderLabel)"
  },
  "calendar.reminders.daysBefore": {
    "message": { "one": "{count} day before at {time}", "other": "{count} days before at {time}" },
    "description": "Reminder choice for an all-day event; count is the number of days before its first day, time a time such as 09:00. src/renderer/calendar-event.js (reminderLabel)"
  },
  "calendar.reminders.weekBefore": {
    "message": "A week before at {time}",
    "description": "Reminder choice for an all-day event: a week before its first day at a time such as 09:00. src/renderer/calendar-event.js (reminderLabel)"
  },
  "calendar.reminders.email": {
    "message": "{when}, by email",
    "description": "Event dialog: an email reminder, shown as it is and kept; Rukoo doesn't change email reminders. when is a reminder label such as '1 day before'. src/renderer/calendar-event.js (remindersHtml)"
  },
  "calendar.reminders.useDefault": {
    "message": "Use the calendar's default reminders",
    "description": "Event dialog checkbox, only for calendars with default reminders (Google); the defaults follow in brackets for timed events. src/renderer/calendar-event.js (remindersHtml)"
  },
  "calendar.reminders.none": {
    "message": "No reminders",
    "description": "Event dialog: the event has no reminders; also in the More section's summary. src/renderer/calendar-event.js (remindersHtml, moreSummary)"
  },
  "calendar.reminders.fallback": {
    "message": "Rukoo reminds you {when}, as set in Settings → Calendar.",
    "description": "Event dialog: a timed event without reminders of its own gets the default reminder from Settings → Calendar; when is a reminder label such as '10 minutes before' (calendar.reminders.minutes). src/renderer/calendar-event.js (remindersHtml)"
  },
  "calendar.reminders.count": {
    "message": { "one": "{count} reminder", "other": "{count} reminders" },
    "description": "Summary after the event dialog's More button; count is the number of reminders. src/renderer/calendar-event.js (moreSummary)"
  },
  "calendar.timezone.label": {
    "message": "Time zone",
    "description": "Event dialog, More section: label of the event's time zone, a field that searches the list of zones as you type. src/renderer/calendar-event.js (formHtml)"
  },
  "calendar.timezone.computer": {
    "message": "{offset}, this computer's time zone",
    "description": "Line under the time zone field when it is the computer's own zone; offset is like GMT+01:00. src/renderer/calendar-event.js (zoneHelp)"
  },
  "calendar.timezone.inZone": {
    "message": "{time} in {zone}",
    "description": "Event dialog: the event's times on its own zone's clock, when that isn't the computer's; time is like 03:00 – 04:00, zone a name such as America/New York. src/renderer/calendar-event.js (zoneNote)"
  },
  "calendar.timezone.invalid": {
    "message": "Choose a time zone from the list.",
    "description": "Event dialog error when the time zone field holds something that isn't a time zone. src/renderer/calendar-event.js (readForm)"
  },
  "calendar.timezone.unknown": {
    "message": "Not a time zone Rukoo knows. It stays as it is unless you choose another.",
    "description": "Line under the time zone field when the event's zone, set in another app, isn't one Rukoo knows. src/renderer/calendar-event.js (zoneHelp)"
  },
  "calendar.showAs.label": {
    "message": "Show as",
    "description": "Event dialog, More section: label of the choice between busy and free. src/renderer/calendar-event.js (formHtml)"
  },
  "calendar.showAs.busy": {
    "message": "Busy",
    "description": "Show-as choice: the event makes the user busy, also for availability and the booking page. src/renderer/calendar-event.js (formHtml, moreSummary)"
  },
  "calendar.showAs.free": {
    "message": "Free",
    "description": "Show-as choice: the event leaves the user free. New all-day events start as free. src/renderer/calendar-event.js (formHtml, moreSummary)"
  },
  "calendar.visibility.label": {
    "message": "Visibility",
    "description": "Event dialog, More section: label of who can see the event's details. src/renderer/calendar-event.js (formHtml)"
  },
  "calendar.visibility.default": {
    "message": "Default visibility",
    "description": "Visibility choice: as the calendar's sharing settings say. src/renderer/calendar-event.js (formHtml)"
  },
  "calendar.visibility.private": {
    "message": "Private",
    "description": "Visibility choice: people the calendar is shared with see only that the time is taken. src/renderer/calendar-event.js (formHtml, moreSummary)"
  },
  "calendar.visibility.public": {
    "message": "Public",
    "description": "Visibility set in another app: anyone who can see the calendar sees the details. Shown only for events that have it. src/renderer/calendar-event.js (formHtml)"
  },
  "calendar.visibility.confidential": {
    "message": "Confidential",
    "description": "Visibility set in another app (CalDAV CLASS:CONFIDENTIAL). Shown only for events that have it. src/renderer/calendar-event.js (formHtml)"
  },
  "calendar.booking.booked": {
    "message": "Booked through your booking page",
    "description": "Event card: first line of an event made from a booking; the meeting type follows under it. src/renderer/calendar-event.js (popHtml)"
  },
  "calendar.booking.noDetails": {
    "message": "The guest's details can't be read on this computer.",
    "description": "Booking or request card when the sealed guest details couldn't be opened here. src/renderer/calendar-event.js (guestRows)"
  },
  "calendar.booking.guestTime": {
    "message": "Guest's time: {time} ({zone})",
    "description": "Booking or request card: the meeting on the guest's clock; time is like 09:00 – 09:30, zone the guest's time zone such as Europe/London. src/renderer/calendar-event.js (guestRows)"
  },
  "calendar.booking.guestZone": {
    "message": "Guest's time zone: {zone}",
    "description": "Booking or request card: the guest's time zone, when it can't be used to show their time. src/renderer/calendar-event.js (guestRows)"
  },
  "calendar.booking.via": {
    "message": "Assistant: {via}",
    "description": "Booking or request card: who made the booking for the guest, as the booking said, such as an assistant's name. via is the guest's own text. src/renderer/calendar-event.js (guestRows)"
  },
  "calendar.booking.customLength": {
    "message": "Length changed in your calendar",
    "description": "Event card of a booked event whose length the host changed; the booking keeps its own length. src/renderer/calendar-event.js (popHtml)"
  },
  "calendar.booking.cancelShortcut": {
    "message": "Cancel booking (Delete)",
    "description": "Tooltip of the event card's delete button on a booked event, on the computer that handles bookings, with its keyboard shortcut. src/renderer/calendar-event.js (popHtml)"
  },
  "calendar.booking.cancelTitle": {
    "message": "Cancel this booking?",
    "description": "Title of the dialog before a booked event is deleted and its booking cancelled. src/renderer/calendar-event.js (cancelBooking)"
  },
  "calendar.booking.cancelHelp": {
    "message": "{name} gets a cancellation for {when}.",
    "description": "Text in the cancel-booking dialog; name is the guest's name, when the event's day and times. src/renderer/calendar-event.js (cancelBooking)"
  },
  "calendar.booking.cancelHelpNoName": {
    "message": "The guest gets a cancellation for {when}.",
    "description": "Text in the cancel-booking dialog when the guest's details can't be read; when is the event's day and times. src/renderer/calendar-event.js (cancelBooking)"
  },
  "calendar.booking.message": {
    "message": "Message to the guest (optional)",
    "description": "Label of the field for a short message to the guest when a booking is cancelled or a request declined. src/renderer/calendar-event.js (cancelBooking, declineRequest)"
  },
  "calendar.booking.messageHelp": {
    "message": "It shows on the guest's confirmation page.",
    "description": "Line under the message field in the cancel-booking dialog. src/renderer/calendar-event.js (cancelBooking)"
  },
  "calendar.booking.keep": {
    "message": "Keep booking",
    "description": "Button in the cancel-booking dialog that closes it without cancelling. src/renderer/calendar-event.js (cancelBooking)"
  },
  "calendar.booking.cancel": {
    "message": "Cancel booking",
    "description": "Button in the cancel-booking dialog that deletes the event and cancels the booking. src/renderer/calendar-event.js (cancelBooking)"
  },
  "calendar.booking.cancelled": {
    "message": "Booking cancelled",
    "description": "Toast after a booking was cancelled from its event card. src/renderer/calendar-event.js (cancelBooking)"
  },
  "calendar.booking.deleteTitle": {
    "message": "Delete this booked event?",
    "description": "Title of the confirmation before a booked event is deleted on a computer that doesn't handle the bookings. src/renderer/calendar-event.js (removeEvent)"
  },
  "calendar.booking.deleteElsewhere": {
    "message": "The booking is cancelled when {device}, which handles your bookings, syncs next.",
    "description": "Text of that confirmation; device is the name of the computer that handles bookings, or calendar.booking.otherComputer. src/renderer/calendar-event.js (removeEvent)"
  },
  "calendar.booking.otherComputer": {
    "message": "your other computer",
    "description": "Stands for the computer that handles bookings when its name isn't known, inside calendar.booking.deleteElsewhere and calendar.booking.requestElsewhere. src/renderer/calendar-event.js (bookingDevice)"
  },
  "calendar.booking.request": {
    "message": "Booking request",
    "description": "Title of a booking request's card when the guest's name can't be read. src/renderer/calendar-event.js (requestHtml)"
  },
  "calendar.booking.requestFrom": {
    "message": "Booking request from {name}",
    "description": "Title of a booking request's card; name is the guest's name. src/renderer/calendar-event.js (requestHtml)"
  },
  "calendar.booking.expires": {
    "message": "Lapses if not answered by {when}",
    "description": "Request card: when the request lapses unanswered; when is a day and time. src/renderer/calendar-event.js (requestHtml)"
  },
  "calendar.booking.accept": {
    "message": "Accept",
    "description": "Request card button that confirms the booking; the event goes into the calendar and the guest gets the invitation. src/renderer/calendar-event.js (requestHtml)"
  },
  "calendar.booking.decline": {
    "message": "Decline",
    "description": "Request card button, and the button in its dialog, that declines the booking request. src/renderer/calendar-event.js (requestHtml, declineRequest)"
  },
  "calendar.booking.accepted": {
    "message": "Request accepted",
    "description": "Toast after a booking request was accepted. src/renderer/calendar-event.js (acceptRequest)"
  },
  "calendar.booking.declined": {
    "message": "Request declined",
    "description": "Toast after a booking request was declined. src/renderer/calendar-event.js (declineRequest)"
  },
  "calendar.booking.declineTitle": {
    "message": "Decline this request?",
    "description": "Title of the dialog before a booking request is declined. src/renderer/calendar-event.js (declineRequest)"
  },
  "calendar.booking.declineHelp": {
    "message": "{name} sees on their confirmation page that the request was declined.",
    "description": "Text in the decline dialog; name is the guest's name. src/renderer/calendar-event.js (declineRequest)"
  },
  "calendar.booking.declineHelpNoName": {
    "message": "The guest sees on their confirmation page that the request was declined.",
    "description": "Text in the decline dialog when the guest's details can't be read. src/renderer/calendar-event.js (declineRequest)"
  },
  "calendar.booking.declineEmail": {
    "message": "Also email the message to the guest",
    "description": "Checkbox in the decline dialog, off by default; the email goes from the booking page's calendar account. src/renderer/calendar-event.js (declineRequest)"
  },
  "calendar.booking.declineEmailFrom": {
    "message": "Also email the message from {address}",
    "description": "As calendar.booking.declineEmail, with the address the email goes from. src/renderer/calendar-event.js (declineRequest)"
  },
  "calendar.booking.requestElsewhere": {
    "message": "{device} handles your bookings. Answer the request there.",
    "description": "Request card on a computer that doesn't handle bookings, instead of Accept and Decline; device is the name of the computer that does, or calendar.booking.otherComputer. src/renderer/calendar-event.js (requestHtml)"
  },
  "calendar.booking.requestGone": {
    "message": "This request was answered or has lapsed.",
    "description": "Toast when a booking request's card is asked for but the request is no longer pending. src/renderer/calendar-event.js (openRequest)"
  },
  "common.actions.add": {
    "message": "Add",
    "description": "common > actions. src/renderer/app.js (messageMenu); src/renderer/settings.js (render)"
  },
  "common.actions.archive": {
    "message": "Archive",
    "description": "common > actions. src/renderer/app.js (renderListTools); src/renderer/app.js (bulkMenu); src/renderer/app.js (messageMenu); src/renderer/app.js (readerBar); src/renderer/app.js (renderReader)"
  },
  "common.actions.back": {
    "message": "Back",
    "description": "common > actions. src/renderer/settings.js (render); src/renderer/setup.js (showGrid); src/renderer/setup.js (showGoogle); src/renderer/setup.js (showLogin)"
  },
  "common.actions.cancel": {
    "message": "Cancel",
    "description": "common > actions. src/renderer/settings.js (promptDialog); src/renderer/settings.js (openSettings); src/renderer/setup.js (showGoogle); src/renderer/composer.js (insertLink); src/renderer/composer.js (close); src/renderer/ui.js (confirmDialog); src/renderer/ui.js (choiceDialog); src/main/main.js"
  },
  "common.actions.clear": {
    "message": "Clear",
    "description": "common > actions. src/renderer/app.js (renderListHead); src/renderer/app.js (itemHtml)"
  },
  "common.actions.close": {
    "message": "Close",
    "description": "common > actions. src/renderer/compose.js (init)"
  },
  "common.actions.delete": {
    "message": "Delete",
    "description": "common > actions. src/renderer/app.js (removeMessages); src/renderer/app.js (messageMenu); src/renderer/app.js (renderReader); src/renderer/settings.js (openSettings); src/renderer/composer.js (recipientsHtml); src/renderer/composer.js (mountComposer); src/renderer/composer.js (discard)"
  },
  "common.actions.moreOptions": {
    "message": "More options",
    "description": "common > actions. src/renderer/app.js (renderListHead); src/renderer/composer.js (template)"
  },
  "common.actions.move": {
    "message": "Move",
    "description": "common > actions. src/renderer/app.js (renderReader)"
  },
  "common.actions.ok": {
    "message": "OK",
    "description": "common > actions. src/renderer/ui.js (dialog); src/renderer/ui.js (confirmDialog)"
  },
  "common.actions.retry": {
    "message": "Retry",
    "description": "common > actions. Tries a failed fetch again. src/renderer/calendar.js (noticesHtml)"
  },
  "common.actions.save": {
    "message": "Save",
    "description": "common > actions. src/renderer/app.js (attachmentsHtml); src/renderer/settings.js (promptDialog); src/renderer/settings.js (render); src/renderer/settings.js (openSettings); src/renderer/composer.js (close)"
  },
  "common.actions.undo": {
    "message": "Undo",
    "description": "common > actions. src/renderer/app.js (offerUndo)"
  },
  "common.dates.today": {
    "message": "Today",
    "description": "common > dates. src/renderer/ui.js (groupLabel)"
  },
  "common.dates.yesterday": {
    "message": "Yesterday",
    "description": "common > dates. src/renderer/ui.js (groupLabel)"
  },
  "common.errors.invalidEmail": {
    "message": "Invalid email address",
    "description": "common > errors. src/renderer/settings.js (openSettings)"
  },
  "common.errors.invalidEmailValue": {
    "message": "Invalid email address: {address}",
    "description": "common > errors. src/renderer/composer.js (send); src/main/engine.js"
  },
  "common.status.saving": {
    "message": "Saving...",
    "description": "Progress status while a draft save is running, shown by src/renderer/composer.js (describeSaved). Separate from the Save action in native file dialogs."
  },
  "common.values.none": {
    "message": "None",
    "description": "common > values. src/renderer/settings.js; src/renderer/settings.js (render)"
  },
  "common.values.noneLower": {
    "message": "none",
    "description": "common > values. Account signature value when no general signature is configured."
  },
  "composer.actions.addBcc": {
    "message": "Add Bcc",
    "description": "composer > actions. src/renderer/composer.js (mountComposer)"
  },
  "composer.actions.addCc": {
    "message": "Add Cc",
    "description": "composer > actions. src/renderer/composer.js (mountComposer)"
  },
  "composer.actions.closeShortcut": {
    "message": "Close (Esc)",
    "description": "composer > actions. src/renderer/composer.js (template)"
  },
  "composer.actions.discard": {
    "message": "Delete draft",
    "description": "composer > actions. src/renderer/composer.js (template); src/renderer/composer.js (mountComposer); src/renderer/composer.js (close)"
  },
  "composer.actions.editDraft": {
    "message": "Edit draft",
    "description": "composer > actions. src/renderer/app.js (messageMenu)"
  },
  "composer.actions.makeDefault": {
    "message": "Use as default account",
    "description": "composer > actions. Small link beside the From field of a new message, shown when the chosen account is not the default; new emails then start from it. src/renderer/composer.js (template)"
  },
  "composer.actions.popout": {
    "message": "Open in a separate window",
    "description": "composer > actions. src/renderer/composer.js (template); src/renderer/composer.js (mountComposer)"
  },
  "composer.actions.saveDraft": {
    "message": "Save to Drafts",
    "description": "composer > actions. src/renderer/composer.js (mountComposer)"
  },
  "composer.actions.send": {
    "message": "Send",
    "description": "composer > actions. src/renderer/composer.js (template); src/renderer/composer.js (send)"
  },
  "composer.actions.sendShortcut": {
    "message": "Send (Ctrl+Enter)",
    "description": "composer > actions. src/renderer/composer.js (template)"
  },
  "composer.agent.drafted": {
    "message": "Drafted by {name}",
    "description": "Composer bar mark after an agent wrote the draft; name is the agent."
  },
  "composer.agent.draftedEdited": {
    "message": "Drafted by {name} · edited",
    "description": "Composer bar mark after an agent wrote the draft and you changed it."
  },
  "composer.agent.someone": {
    "message": "an agent",
    "description": "Composer bar mark: fallback when the agent has no name (\"Drafted by an agent\")."
  },
  "composer.agent.undo": {
    "message": "Undo",
    "description": "Composer bar link that restores what was there before the agent's draft."
  },
  "composer.agent.undoHelp": {
    "message": "Your own changes since then are undone too.",
    "description": "Composer confirmation text when undoing an agent draft you already edited."
  },
  "composer.agent.undoTitle": {
    "message": "Undo {name}'s draft?",
    "description": "Composer confirmation title when undoing an agent draft you already edited."
  },
  "composer.attachments.image": {
    "message": "Insert image",
    "description": "composer > attachments. src/renderer/composer.js (template); src/main/main.js"
  },
  "composer.attachments.pick": {
    "message": "Attach files",
    "description": "composer > attachments. src/renderer/composer.js (template); src/main/main.js"
  },
  "composer.body.label": {
    "message": "Message body",
    "description": "composer > body. src/renderer/composer.js (template)"
  },
  "composer.body.placeholder": {
    "message": "Write your message",
    "description": "composer > body. src/renderer/composer.js (template)"
  },
  "composer.close.discard": {
    "message": "Don't save",
    "description": "composer > close. src/renderer/composer.js (close)"
  },
  "composer.close.help": {
    "message": "Save this message to Drafts to continue writing later.",
    "description": "composer > close. src/renderer/composer.js (close)"
  },
  "composer.close.title": {
    "message": "Save draft?",
    "description": "composer > close. src/renderer/composer.js (close)"
  },
  "composer.discard.savedHelp": {
    "message": "This draft will also be removed from Drafts.",
    "description": "composer > discard. src/renderer/composer.js (discard)"
  },
  "composer.discard.title": {
    "message": "Delete draft?",
    "description": "composer > discard. src/renderer/composer.js (discard)"
  },
  "composer.discard.unsavedHelp": {
    "message": "What you have written will be lost.",
    "description": "composer > discard. src/renderer/composer.js (discard)"
  },
  "composer.edit.cut": {
    "message": "Cut",
    "description": "Menu item: cuts the selected text to the clipboard. Context menu on the message text in the composer (right-click, the context menu key or Shift+F10). src/renderer/editmenu.js (open)"
  },
  "composer.edit.copy": {
    "message": "Copy",
    "description": "Menu item: copies the selected text to the clipboard. Context menu on the message text in the composer. src/renderer/editmenu.js (open)"
  },
  "composer.edit.paste": {
    "message": "Paste",
    "description": "Menu item: pastes what is on the clipboard, with its formatting. Context menu on the message text in the composer. src/renderer/editmenu.js (open)"
  },
  "composer.edit.pastePlain": {
    "message": "Paste as plain text",
    "description": "Menu item: pastes the clipboard's text without its formatting, so it takes on the style of the text around it. Context menu on the message text in the composer. src/renderer/editmenu.js (open)"
  },
  "composer.edit.selectAll": {
    "message": "Select all",
    "description": "Menu item: selects all of the message text. Context menu on the message text in the composer. src/renderer/editmenu.js (open)"
  },
  "composer.errors.autosave": {
    "message": "Autosave failed",
    "description": "composer > errors. src/renderer/composer.js (saveDraft)"
  },
  "composer.errors.missingAttachments": {
    "message": "The draft is in Drafts, but its attachments could not be found. Close this window and reopen the draft.",
    "description": "composer > errors. src/renderer/composer.js (settleAttachments)"
  },
  "composer.errors.noAccount": {
    "message": "Add an email account in Rukoo Mail before you write an email.",
    "description": "composer > errors. Shown in a compose window opened before any account exists, by a mailto: link from another app or by New message in the tray. src/renderer/compose.js (init)"
  },
  "composer.errors.noRecipients": {
    "message": "Add at least one recipient",
    "description": "composer > errors. src/renderer/composer.js (send)"
  },
  "composer.errors.send": {
    "message": "Sending failed",
    "description": "composer > errors. src/renderer/composer.js (send)"
  },
  "composer.format.bold": {
    "message": "Bold (Ctrl+B)",
    "description": "composer > format. src/renderer/composer.js"
  },
  "composer.format.bullets": {
    "message": "Bulleted list",
    "description": "composer > format. src/renderer/composer.js"
  },
  "composer.format.clear": {
    "message": "Clear formatting",
    "description": "composer > format. src/renderer/composer.js (template)"
  },
  "composer.format.highlight": {
    "message": "Highlight",
    "description": "composer > format. src/renderer/composer.js (template)"
  },
  "composer.format.indent": {
    "message": "Increase indent",
    "description": "composer > format. src/renderer/composer.js (template)"
  },
  "composer.format.italic": {
    "message": "Italic (Ctrl+I)",
    "description": "composer > format. src/renderer/composer.js"
  },
  "composer.format.label": {
    "message": "Formatting",
    "description": "composer > format. src/renderer/composer.js (template)"
  },
  "composer.format.link": {
    "message": "Link (Ctrl+K)",
    "description": "composer > format. src/renderer/composer.js"
  },
  "composer.format.numbered": {
    "message": "Numbered list",
    "description": "composer > format. src/renderer/composer.js"
  },
  "composer.format.outdent": {
    "message": "Decrease indent",
    "description": "composer > format. src/renderer/composer.js (template)"
  },
  "composer.format.textColor": {
    "message": "Text color",
    "description": "composer > format. src/renderer/composer.js (template)"
  },
  "composer.format.underline": {
    "message": "Underline (Ctrl+U)",
    "description": "composer > format. src/renderer/composer.js"
  },
  "composer.link.address": {
    "message": "Address",
    "description": "composer > link. src/renderer/composer.js (insertLink)"
  },
  "composer.link.insert": {
    "message": "Insert",
    "description": "composer > link. src/renderer/composer.js (insertLink)"
  },
  "composer.link.title": {
    "message": "Insert link",
    "description": "composer > link. src/renderer/composer.js (insertLink)"
  },
  "composer.mentions.addedCc": {
    "message": "{name} added to Cc",
    "description": "Toast after picking someone to @-mention who wasn't a recipient yet: the composer adds them to Cc. {name} is the person's name, or their email address when there is none. src/renderer/mentions.js (pick)"
  },
  "composer.mentions.addsCc": {
    "message": "Adds to Cc",
    "description": "Read by screen readers for a person in the @-mention suggestions who isn't a recipient yet: picking them adds them to Cc. src/renderer/mentions.js (draw)"
  },
  "composer.mentions.hint": {
    "message": "People who aren't on the email yet go to Cc",
    "description": "Quiet line under the @-mention suggestions in the composer, shown while picking someone would add them as a recipient. src/renderer/mentions.js (draw)"
  },
  "composer.mentions.inBcc": {
    "message": "Already in Bcc",
    "description": "Tooltip of the Bcc tag on a person in the @-mention suggestions who is already a Bcc recipient, and read by screen readers. Picking them only inserts the mention. src/renderer/mentions.js (draw)"
  },
  "composer.mentions.inCc": {
    "message": "Already in Cc",
    "description": "Tooltip of the Cc tag on a person in the @-mention suggestions who is already a Cc recipient, and read by screen readers. Picking them only inserts the mention. src/renderer/mentions.js (draw)"
  },
  "composer.mentions.inTo": {
    "message": "Already in To",
    "description": "Tooltip of the To tag on a person in the @-mention suggestions who is already a To recipient, and read by screen readers. Picking them only inserts the mention. src/renderer/mentions.js (draw)"
  },
  "composer.mentions.label": {
    "message": "People to mention",
    "description": "Accessible name of the suggestion list that opens when you type @ and a name in the composer's text. src/renderer/mentions.js (draw)"
  },
  "composer.quote.date": {
    "message": "Date: {date}",
    "description": "composer > quote. src/renderer/composer.js (quoteHeader)"
  },
  "composer.quote.exclude": {
    "message": "Exclude from message",
    "description": "composer > quote. src/renderer/composer.js (template); src/renderer/composer.js (mountComposer)"
  },
  "composer.quote.from": {
    "message": "From: {sender}",
    "description": "composer > quote. src/renderer/composer.js (quoteHeader)"
  },
  "composer.quote.hide": {
    "message": "Hide previous messages",
    "description": "composer > quote. src/renderer/composer.js (mountComposer)"
  },
  "composer.quote.include": {
    "message": "Include in message",
    "description": "composer > quote. src/renderer/composer.js (mountComposer)"
  },
  "composer.quote.marker": {
    "message": "-------- Original message --------",
    "description": "composer > quote. Header inserted above quoted email when replying or forwarding."
  },
  "composer.quote.show": {
    "message": "Show previous messages",
    "description": "composer > quote. src/renderer/composer.js (template); src/renderer/composer.js (mountComposer)"
  },
  "composer.quote.subject": {
    "message": "Subject: {subject}",
    "description": "composer > quote. src/renderer/composer.js (quoteHeader)"
  },
  "composer.quote.title": {
    "message": "Previous message",
    "description": "composer > quote. src/renderer/composer.js (template)"
  },
  "composer.quote.to": {
    "message": "To: {recipients}",
    "description": "composer > quote. src/renderer/composer.js (quoteHeader)"
  },
  "composer.send.noSubjectHelp": {
    "message": "Send this email without a subject?",
    "description": "composer > send. src/renderer/composer.js (send)"
  },
  "composer.send.noSubjectTitle": {
    "message": "No subject",
    "description": "composer > send. src/renderer/composer.js (send)"
  },
  "composer.spelling.add": {
    "message": "Add to dictionary",
    "description": "Menu item: adds the right-clicked misspelled word to the spell checker's dictionary, so it is no longer underlined. Context menu on a misspelled word in the composer, under the spelling suggestions. src/renderer/editmenu.js (open)"
  },
  "composer.spelling.none": {
    "message": "No suggestions",
    "description": "Disabled menu row where the spelling suggestions go, when the spell checker has none for the right-clicked misspelled word. Context menu on a misspelled word in the composer. src/renderer/editmenu.js (open)"
  },
  "composer.status.defaultAccount": {
    "message": "{address} is now the default account",
    "description": "composer > status. Toast after composer.actions.makeDefault. {address} is the account's email address. src/renderer/composer.js (makeDefault)"
  },
  "composer.status.deleted": {
    "message": "Draft deleted",
    "description": "composer > status. src/renderer/composer.js (discard); src/renderer/composer.js (close)"
  },
  "composer.status.kept": {
    "message": "Draft kept in Drafts",
    "description": "composer > status. src/renderer/composer.js (close); src/renderer/composer.js (leave); src/renderer/composer.js (mountComposer)"
  },
  "composer.status.saved": {
    "message": "Draft saved",
    "description": "composer > status. src/renderer/composer.js (mountComposer)"
  },
  "composer.status.savedAt": {
    "message": "Saved at {time}",
    "description": "composer > status. src/renderer/composer.js (mountComposer)"
  },
  "composer.status.savedClosed": {
    "message": "Draft saved to Drafts",
    "description": "composer > status. src/renderer/composer.js (leave)"
  },
  "composer.status.savedToDrafts": {
    "message": "Saved to Drafts",
    "description": "composer > status. src/renderer/composer.js (saveDraft); src/renderer/composer.js (close)"
  },
  "composer.status.sending": {
    "message": "Sending...",
    "description": "composer > status. src/renderer/composer.js (send)"
  },
  "composer.status.sent": {
    "message": "Email sent",
    "description": "composer > status. src/renderer/composer.js (send); src/main/main.js"
  },
  "composer.titles.draft": {
    "message": "Draft",
    "description": "composer > titles. src/renderer/app.js (itemHtml); src/renderer/app.js (renderReader); src/renderer/composer.js"
  },
  "composer.titles.forward": {
    "message": "Forward",
    "description": "composer > titles. src/renderer/app.js (messageMenu); src/renderer/composer.js"
  },
  "composer.titles.new": {
    "message": "New message",
    "description": "composer > titles. src/renderer/app.js (renderSidebar); src/renderer/app.js (personMenu); src/renderer/composer.js; src/renderer/composer.js (template); src/renderer/composer.js (mountComposer); src/main/main.js (openComposeWindow)"
  },
  "composer.titles.newShortcut": {
    "message": "New message (Ctrl+N)",
    "description": "composer > titles. src/renderer/app.js (renderSidebar)"
  },
  "composer.titles.reply": {
    "message": "Reply",
    "description": "composer > titles. src/renderer/app.js (messageMenu); src/renderer/app.js (readerBar); src/renderer/composer.js"
  },
  "composer.titles.replyAll": {
    "message": "Reply all",
    "description": "composer > titles. src/renderer/app.js (messageMenu); src/renderer/composer.js"
  },
  "errors.account.duplicate": {
    "message": "This account has already been added.",
    "description": "errors > account. src/main/engine.js"
  },
  "errors.account.identity": {
    "message": "This address does not belong to this account.",
    "description": "errors > account. src/main/engine.js"
  },
  "errors.account.invalidEmail": {
    "message": "Enter a valid email address.",
    "description": "errors > account. src/main/engine.js"
  },
  "errors.account.missing": {
    "message": "Account not found.",
    "description": "errors > account. src/main/engine.js"
  },
  "errors.account.password": {
    "message": "Enter your password.",
    "description": "errors > account. src/main/engine.js"
  },
  "errors.attachment.missing": {
    "message": "Attachment not found.",
    "description": "errors > attachment. src/main/engine.js"
  },
  "errors.image.unavailable": {
    "message": "Rukoo couldn't get this image.",
    "description": "Context menu on an image in an email: the image could not be downloaded, or is larger than 25 MB. src/main/mailimage.js"
  },
  "errors.image.unsupported": {
    "message": "Rukoo can't copy or save this kind of image.",
    "description": "Context menu on an image in an email: the image is not a PNG, JPEG, GIF, WebP, BMP or ICO file (an SVG, for example). src/main/mailimage.js"
  },
  "errors.connection.auth": {
    "message": "Sign-in failed. Check your email address and password or app password.",
    "description": "errors > connection. src/main/imap.js (friendlyError)"
  },
  "errors.connection.certificate": {
    "message": "The server certificate is not trusted.",
    "description": "errors > connection. src/main/imap.js (friendlyError)"
  },
  "errors.connection.closed": {
    "message": "The connection to the mail server was closed. Try again.",
    "description": "errors > connection. A mail command that was still waiting when the account's connection closed, because the account was removed, signed in again or moved to another server. Shown in the reader or a toast. src/main/imap.js (ImapAccount.drain)"
  },
  "errors.connection.googleAppPassword": {
    "message": "Google didn't accept this password. Gmail only works here with an app password, not your normal Google password. Create one and try again.",
    "description": "errors > connection. Shown in the setup form and under Server settings when Gmail turns down the password of an account that signs in with a password. The setup form shows a Create app password link under it. src/main/imap.js (friendlyError, verifyError)"
  },
  "errors.connection.host": {
    "message": "Server not found. Check the server name.",
    "description": "errors > connection. src/main/imap.js (friendlyError)"
  },
  "errors.connection.refused": {
    "message": "Connection refused. Check the server and port.",
    "description": "errors > connection. src/main/imap.js (friendlyError)"
  },
  "errors.connection.timeout": {
    "message": "The connection to the server timed out.",
    "description": "errors > connection. src/main/imap.js (friendlyError)"
  },
  "errors.connection.yahooAppPassword": {
    "message": "Yahoo didn't accept this password. Yahoo Mail only works here with an app password, not your normal Yahoo password. Create one and try again.",
    "description": "errors > connection. Shown in the setup form and under Server settings when Yahoo turns down the password of an account. The setup form shows a Create app password link under it. src/main/imap.js (friendlyError, verifyError)"
  },
  "errors.folder.duplicate": {
    "message": "A folder with that name already exists.",
    "description": "errors > folder. src/main/engine.js"
  },
  "errors.folder.invalidName": {
    "message": "A folder name cannot contain / \\ % or *.",
    "description": "errors > folder. src/main/engine.js"
  },
  "errors.folder.nameRequired": {
    "message": "Enter a folder name.",
    "description": "errors > folder. src/main/engine.js"
  },
  "errors.folder.noArchive": {
    "message": "This account has no archive folder.",
    "description": "errors > folder. src/main/engine.js"
  },
  "errors.folder.noDrafts": {
    "message": "This account has no Drafts folder.",
    "description": "errors > folder. src/main/engine.js"
  },
  "errors.folder.rebuilt": {
    "message": "The folder was rebuilt on the server.",
    "description": "errors > folder. src/main/imap.js"
  },
  "errors.calendar.apiDisabled": {
    "message": "The Google Calendar API is turned off in the Google Cloud project of your OAuth client. Turn it on there, then try again.",
    "description": "Calendar notice when Google answers that the Calendar API isn't enabled for the user's own OAuth client. src/main/calendar/google.js"
  },
  "errors.calendar.busy": {
    "message": "Google Calendar is busy. Try again in a minute.",
    "description": "Calendar error when Google limits the number of requests. src/main/calendar/google.js"
  },
  "errors.calendar.changed": {
    "message": "This event was changed elsewhere after Rukoo loaded it, so Rukoo left it as it is.",
    "description": "Calendar error when a change or deletion was refused because the event changed in Google Calendar since Rukoo loaded it (HTTP 412), or Rukoo had no version of it to check. Shown in a toast after Delete, and in the event dialog when the latest version couldn't be loaded. src/main/calendar/google.js; src/main/calendar/index.js (unlessChanged)"
  },
  "errors.calendar.invalidResponse": {
    "message": "That isn't an answer to an invitation.",
    "description": "Calendar error when an RSVP has a value other than yes, maybe or no. src/main/calendar/index.js"
  },
  "errors.calendar.invalidTimes": {
    "message": "An event needs a start before its end.",
    "description": "Calendar error when a new or changed event has missing or reversed times. src/main/calendar/index.js"
  },
  "errors.calendar.noCalendar": {
    "message": "This calendar can't be found, or it doesn't take new events.",
    "description": "Calendar error when a new event names a calendar that is gone or read-only. src/main/calendar/index.js"
  },
  "errors.calendar.notFound": {
    "message": "This event no longer exists.",
    "description": "Calendar error when the event was deleted elsewhere in the meantime. src/main/calendar/google.js; src/main/calendar/index.js"
  },
  "errors.calendar.notInvited": {
    "message": "You aren't a guest of this event.",
    "description": "Calendar error when the user answers an event they aren't invited to. src/main/calendar/google.js; src/main/calendar/index.js"
  },
  "errors.calendar.readOnly": {
    "message": "You can't change this event.",
    "description": "Calendar error when the user changes an event on a read-only calendar or one organized by someone else. src/main/calendar/index.js"
  },
  "errors.calendar.request": {
    "message": "Google Calendar refused the request ({reason}).",
    "description": "Calendar error for any other refusal; reason is Google's own message or the HTTP status. src/main/calendar/google.js"
  },
  "errors.calendar.scope": {
    "message": "This account hasn't given Rukoo access to its calendar.",
    "description": "Calendar error when the Google grant doesn't cover the calendar. src/main/calendar/google.js"
  },
  "errors.calendar.signIn": {
    "message": "Google turned down the sign-in. Connect the calendar again.",
    "description": "Calendar error when Google rejects the account's access token. src/main/calendar/google.js"
  },
  "errors.calendar.tooMany": {
    "message": "This calendar has more events in this period than Rukoo can fetch.",
    "description": "Calendar error when Google keeps returning pages of events past Rukoo's limit, so the list would be incomplete. src/main/calendar/google.js"
  },
  "errors.calendar.unreachable": {
    "message": "Can't reach Google Calendar. Check your internet connection.",
    "description": "Calendar error when the request to Google fails or times out. src/main/calendar/google.js"
  },
  "errors.calendar.invalidRule": {
    "message": "Rukoo can't save this repeat rule.",
    "description": "Calendar error when an event's repeat rule isn't one Rukoo writes: an unknown part, COUNT together with UNTIL, or BYSETPOS without days. src/main/calendar/index.js (clean)"
  },
  "errors.calendar.invalidZone": {
    "message": "Rukoo doesn't know that time zone.",
    "description": "Calendar error when an event or a free time request names a time zone that isn't a known IANA zone. src/main/calendar/index.js"
  },
  "errors.calendar.invalidGuests": {
    "message": "One of the guests doesn't have a valid email address.",
    "description": "Calendar error when a guest added to, removed from or marked optional on an event has no valid address, or the list is too long. src/main/calendar/index.js (clean)"
  },
  "errors.calendar.invalidReminders": {
    "message": "Rukoo can't save these reminders. An event takes up to 5, each between its start and 4 weeks before it.",
    "description": "Calendar error when an event's reminders are more than 5, too early, or set after the start of a timed event or on the day of a Google all-day event. src/main/calendar/index.js, google.js"
  },
  "errors.calendar.invalidQuery": {
    "message": "Type something to search for, up to 200 characters.",
    "description": "Calendar search error when the search text is empty or longer than 200 characters. src/main/calendar/index.js (search)"
  },
  "errors.calendar.seriesDayChange": {
    "message": "This series repeats on several days, so it can't move to another day as a whole. Change its repeat days instead.",
    "description": "Calendar error when the user moves all events of a series that repeats on several weekdays to another day. src/main/calendar/google.js (moveRule)"
  },
  "errors.calendar.guestListPartial": {
    "message": "The calendar didn't give Rukoo the full guest list, so the guests can't be changed here.",
    "description": "Calendar error when Google leaves guests out of an event even when asked for all of them, so editing the list would drop them. src/main/calendar/google.js, index.js"
  },
  "errors.calendar.notOrganizer": {
    "message": "Only the organizer can make this change.",
    "description": "Calendar error when someone who isn't the organizer changes the guests, or changes or deletes this and following events of a series. src/main/calendar/index.js"
  },
  "errors.calendar.cannotMove": {
    "message": "This event can't move to another calendar. Only your own ordinary events can.",
    "description": "Calendar error when the user moves an event to another calendar that they don't organize, or that Google made itself (a birthday, a flight from Gmail). src/main/calendar/index.js (move)"
  },
  "errors.calendar.noMeet": {
    "message": "This calendar can't add Google Meet.",
    "description": "Calendar error when Google Meet is asked for on a calendar whose account doesn't allow it, or that isn't a Google calendar. src/main/calendar/index.js"
  },
  "errors.calendar.ruleKept": {
    "message": "A new repeat rule applies to this and following events, or to all events, not to one.",
    "description": "Calendar error when the user changes the repeat rule but chose to change only this occurrence of a series. src/main/calendar/index.js"
  },
  "errors.calendar.noGuests": {
    "message": "This calendar doesn't send invitations, so its events can't have guests.",
    "description": "Calendar error when guests are added to an event on a calendar that sends no invitations (the demo account, or a CalDAV server that doesn't schedule). src/main/calendar/index.js"
  },
  "errors.calendar.splitFailed": {
    "message": "Rukoo couldn't change this and the following events, so it left the series as it was. Try again.",
    "description": "Calendar error when splitting a series failed halfway and Rukoo deleted the new part again, so the series is unchanged. src/main/calendar/google.js (split)"
  },
  "errors.calendar.splitUnsure": {
    "message": "The calendar didn't answer, so Rukoo doesn't know whether this and the following events changed. The next sync shows how they are now.",
    "description": "Calendar error when a change or deletion of this and the following events of a series got no clear answer and Rukoo couldn't check afterwards; nothing was undone. src/main/calendar/google.js and caldav.js (split, cut)"
  },
  "errors.calendar.invalidFree": {
    "message": "Rukoo looks for free time of 5 minutes to 8 hours, over at most 31 days.",
    "description": "Free time error when the availability panel, the composer or an agent asks for a length or a period outside those limits. src/main/calendar/free.js"
  },
  "errors.caldav.signIn": {
    "message": "The calendar server turned down the username or password. Check them in Server settings.",
    "description": "Calendar error when the CalDAV server answers 401 to the account's username and password. Shown in the calendar notice and under the Server settings form. src/main/calendar/caldav.js"
  },
  "errors.caldav.setupSignIn": {
    "message": "The calendar server turned down the username or password. Check them, or leave the calendar server empty to add the account without its calendar.",
    "description": "Setup form error when the CalDAV server entered while adding an account answers 401 to its username and password; the account isn't saved. src/main/engine.js (setupCalDav)"
  },
  "errors.caldav.unreachable": {
    "message": "Can't reach the calendar server. Check its address and your internet connection.",
    "description": "Calendar error when a request to the CalDAV server fails or times out. src/main/calendar/caldav.js"
  },
  "errors.caldav.busy": {
    "message": "The calendar server is busy. Try again in a minute.",
    "description": "Calendar error when the CalDAV server answers 429 or 503. src/main/calendar/caldav.js"
  },
  "errors.caldav.request": {
    "message": "The calendar server refused the request ({reason}).",
    "description": "Calendar error for any other refusal by the CalDAV server; reason is the HTTP status code. src/main/calendar/caldav.js"
  },
  "errors.caldav.badAnswer": {
    "message": "The calendar server sent an answer Rukoo can't read.",
    "description": "Calendar error when the CalDAV server answers with something other than the expected XML or iCalendar data. src/main/calendar/caldav.js"
  },
  "errors.caldav.notFound": {
    "message": "No CalDAV server answers at this address. Enter the CalDAV address your provider gives.",
    "description": "Server settings error when neither the address the user entered nor its /.well-known/caldav leads to a CalDAV calendar home. src/main/calendar/caldav.js (discover)"
  },
  "errors.caldav.noCalendars": {
    "message": "The calendar server has no calendars with events for this account.",
    "description": "Server settings error when the CalDAV server answers but lists no calendar that holds events (only task lists, for example). src/main/calendar/caldav.js (discoverCalDav)"
  },
  "errors.caldav.insecure": {
    "message": "Use an https:// address. Rukoo only sends your password over an encrypted connection.",
    "description": "Server settings error when the CalDAV address uses http:// for a server that is not on this PC. src/main/calendar/caldav.js (serverUrl)"
  },
  "errors.caldav.invalidUrl": {
    "message": "That isn't a web address.",
    "description": "Server settings error when the CalDAV address can't be read as an http or https address. src/main/calendar/caldav.js (serverUrl)"
  },
  "errors.caldav.elsewhere": {
    "message": "The calendar server sent Rukoo on to {host}. Rukoo only gives your password to the server you entered; enter {host} in Server settings if you trust it.",
    "description": "Calendar and Server settings error when the CalDAV server redirects, or points its calendars, to a host other than the one the user entered; host is that host name. src/main/calendar/caldav.js (request, homeFrom)"
  },
  "errors.caldav.reconfigured": {
    "message": "The calendar server in Server settings changed while this was on its way. Try again.",
    "description": "Calendar error when a request to the old CalDAV server was still running after the user entered another server in Server settings; Rukoo doesn't send the new password to the old server. src/main/calendar/index.js (provider)"
  },
  "settings.server.invites": {
    "message": "Invitations to guests",
    "description": "Server settings form: label of the select, under the CalDAV address field, that says whether the calendar server emails invitations to the guests the user adds. Shown once a calendar server is set. src/renderer/settings.js (render)"
  },
  "settings.server.invitesAuto": {
    "message": "Automatic",
    "description": "Server settings form, invitations select: option that leaves it to what the calendar server says in its DAV header, before Rukoo has read the server's calendars. src/renderer/settings.js (render)"
  },
  "settings.server.invitesAutoProvider": {
    "message": "Automatic: the server sends them",
    "description": "Server settings form, invitations select: the automatic option once Rukoo knows the calendar server says it emails invitations itself. src/renderer/settings.js (render)"
  },
  "settings.server.invitesAutoNone": {
    "message": "Automatic: the server doesn't send them",
    "description": "Server settings form, invitations select: the automatic option once Rukoo knows the calendar server says it doesn't email invitations. src/renderer/settings.js (render)"
  },
  "settings.server.invitesProvider": {
    "message": "The server sends them",
    "description": "Server settings form, invitations select: option for a calendar server that emails invitations although it doesn't say so; guests can then be added. src/renderer/settings.js (render)"
  },
  "settings.server.invitesNone": {
    "message": "The server doesn't send them",
    "description": "Server settings form, invitations select: option for a calendar server that says it emails invitations but doesn't; events with guests then stay read-only. src/renderer/settings.js (render)"
  },
  "settings.server.invitesHelp": {
    "message": "You can only add guests when your calendar server emails their invitations. Servers say whether they do; choose here if yours gets it wrong.",
    "description": "Server settings form: help text under the invitations select. src/renderer/settings.js (render)"
  },
  "errors.google.aliases": {
    "message": "Gmail did not return aliases ({status}).",
    "description": "errors > google. src/main/engine.js"
  },
  "errors.google.aliasesAuth": {
    "message": "Only available for accounts using Google sign-in.",
    "description": "errors > google. src/main/engine.js"
  },
  "errors.google.cancelled": {
    "message": "Sign-in cancelled.",
    "description": "errors > google. src/main/google.js"
  },
  "errors.google.connection": {
    "message": "Cannot reach Google. Check your internet connection.",
    "description": "errors > google. src/main/google.js"
  },
  "errors.google.denied": {
    "message": "You did not grant access.",
    "description": "errors > google. src/main/google.js"
  },
  "errors.google.expired": {
    "message": "Your Google access has expired or been revoked. Sign in again through Settings.",
    "description": "errors > google. src/main/google.js"
  },
  "errors.google.invalidClient": {
    "message": "This file does not contain a Google OAuth client.",
    "description": "errors > google. src/main/google.js"
  },
  "errors.google.noEmail": {
    "message": "Google did not return an email address.",
    "description": "errors > google. src/main/google.js"
  },
  "errors.google.noRefreshToken": {
    "message": "Google did not return a refresh token. Try again.",
    "description": "errors > google. src/main/google.js"
  },
  "errors.google.notConfigured": {
    "message": "Google sign-in is not configured on this PC.",
    "description": "errors > google. src/main/google.js"
  },
  "errors.google.request": {
    "message": "Google rejected the request ({reason}).",
    "description": "errors > google. src/main/google.js"
  },
  "errors.google.scope": {
    "message": "Allow Rukoo Mail to access Gmail (select all permissions) and try again.",
    "description": "errors > google. src/main/google.js"
  },
  "errors.google.timeout": {
    "message": "Sign-in took too long. Try again.",
    "description": "errors > google. src/main/google.js"
  },
  "errors.google.unavailable": {
    "message": "Google sign-in is unavailable.",
    "description": "errors > google. src/main/engine.js"
  },
  "errors.google.unreachable": {
    "message": "Cannot reach Gmail.",
    "description": "errors > google. src/main/engine.js"
  },
  "errors.google.wrongAccount": {
    "message": "You signed in as {signedInEmail}, but this account is {accountEmail}.",
    "description": "errors > google. src/main/engine.js"
  },
  "errors.video.connection": {
    "message": "Rukoo couldn't reach {service}. Check your internet connection and try again.",
    "description": "A video service didn't answer, while signing in or making a booking's meeting. service is Zoom, Microsoft Teams or Jitsi Meet. src/main/video/oauth.js, src/main/video/index.js"
  },
  "errors.video.cancelled": {
    "message": "Signing in to {service} was cancelled.",
    "description": "The user cancelled the browser sign-in to a video service in Settings > Video meetings. service is Zoom or Microsoft Teams. src/main/video/oauth.js"
  },
  "errors.video.denied": {
    "message": "{service} didn't give Rukoo access.",
    "description": "The user declined access on the video service's sign-in page. service is Zoom or Microsoft Teams. src/main/video/oauth.js"
  },
  "errors.video.timeout": {
    "message": "Signing in to {service} took too long. Try again.",
    "description": "The browser sign-in to a video service wasn't finished within five minutes. service is Zoom or Microsoft Teams. src/main/video/oauth.js"
  },
  "errors.video.notConfigured": {
    "message": "{service} needs an OAuth app before you can connect it. The booking page guide explains how to add one.",
    "description": "Connect was asked for a video service without an OAuth client in this copy of Rukoo. service is Zoom or Microsoft Teams. The guide is docs/booking.md. src/main/video/index.js"
  },
  "errors.video.invalidClient": {
    "message": "This file has no client_id for {service}.",
    "description": "The client file chosen in Settings > Video meetings doesn't hold an OAuth client ID. client_id is the JSON field name; keep it as is. service is Zoom or Microsoft Teams. src/main/video/index.js"
  },
  "errors.video.noRefreshToken": {
    "message": "{service} didn't give Rukoo lasting access. Try connecting again.",
    "description": "The video service's sign-in returned no refresh token, so Rukoo couldn't make meetings later. service is Zoom or Microsoft Teams. src/main/video/index.js"
  },
  "errors.video.expired": {
    "message": "The {service} connection needs renewing in Settings → Video meetings.",
    "description": "The stored sign-in to a video service was refused, expired or revoked. service is Zoom or Microsoft Teams. src/main/video/index.js"
  },
  "errors.video.notConnected": {
    "message": "{service} isn't connected on this computer.",
    "description": "A booking's meeting type uses a video service that isn't connected on the computer that handles bookings. service is Zoom, Microsoft Teams or Jitsi Meet. src/main/video/index.js"
  },
  "errors.video.gone": {
    "message": "{service} no longer has this meeting.",
    "description": "A booking's video meeting was deleted in the service itself before Rukoo could move it. service is Zoom or Microsoft Teams. src/main/video/index.js"
  },
  "errors.video.refused": {
    "message": "{service} refused the request: {reason}",
    "description": "A video service turned down making, moving or deleting a booking's meeting. service is Zoom or Microsoft Teams; reason is the service's own message, often in English. src/main/video/index.js"
  },
  "errors.video.request": {
    "message": "{service} answered with an error: {reason}",
    "description": "A video service failed while signing in or handling a booking's meeting. service is Zoom or Microsoft Teams; reason is the service's own message or an HTTP status. src/main/video/oauth.js, src/main/video/index.js"
  },
  "errors.video.noLink": {
    "message": "{service} made the meeting but sent no link for it.",
    "description": "A video service's answer to making a meeting had no join link. service is Zoom or Microsoft Teams. src/main/video/index.js"
  },
  "errors.video.jitsiServer": {
    "message": "Enter the address of a Jitsi server, starting with https://.",
    "description": "Settings > Video meetings: the Jitsi server address entered isn't an https address. src/main/video/jitsi.js"
  },
  "errors.video.unknown": {
    "message": "Rukoo doesn't know this video service.",
    "description": "A window or a booking named a video service this version of Rukoo doesn't have. src/main/video/index.js"
  },
  "errors.message.missing": {
    "message": "Message not found.",
    "description": "errors > message. src/main/engine.js"
  },
  "errors.message.savedMissing": {
    "message": "Saved email not found.",
    "description": "errors > message. src/main/engine.js"
  },
  "errors.message.serverMissing": {
    "message": "Message not found on the server.",
    "description": "errors > message. src/main/imap.js"
  },
  "errors.network.privateHost": {
    "message": "{hostname} is not a public address",
    "description": "errors > network. src/main/net.js (publicLookup)"
  },
  "errors.send.identity": {
    "message": "You cannot send as {address} from this account.",
    "description": "errors > send. src/main/engine.js"
  },
  "errors.send.noRecipients": {
    "message": "Add at least one recipient.",
    "description": "errors > send. src/main/engine.js"
  },
  "errors.undo.expired": {
    "message": "This action can no longer be undone.",
    "description": "errors > undo. src/main/engine.js"
  },
  "errors.undo.rebuilt": {
    "message": "This action can no longer be undone: the folder was rebuilt on the server.",
    "description": "errors > undo. src/main/engine.js"
  },
  "errors.unsubscribe.missing": {
    "message": "This email has no unsubscribe link.",
    "description": "errors > unsubscribe. src/main/main.js"
  },
  "mailbox.actions.addStar": {
    "message": "Add star",
    "description": "mailbox > actions. src/renderer/app.js (itemHtml); src/renderer/app.js (bulkMenu); src/renderer/app.js (messageMenu); src/renderer/app.js (renderReader)"
  },
  "mailbox.actions.alreadyRead": {
    "message": "All emails are already read",
    "description": "mailbox > actions. src/renderer/app.js (markAllRead)"
  },
  "mailbox.actions.compact": {
    "message": "Compact view",
    "description": "mailbox > actions. src/renderer/app.js (listAction)"
  },
  "mailbox.actions.deleteShortcut": {
    "message": "Delete (Delete)",
    "description": "mailbox > actions. src/renderer/app.js (renderListTools); src/renderer/app.js (readerBar)"
  },
  "mailbox.actions.markedUnread": {
    "message": "Marked as unread",
    "description": "mailbox > actions. src/renderer/app.js (bindReader)"
  },
  "mailbox.actions.moveMenu": {
    "message": "Move...",
    "description": "mailbox > actions. src/renderer/app.js (bulkMenu); src/renderer/app.js (messageMenu)"
  },
  "mailbox.actions.moveShortcut": {
    "message": "Move (Ctrl+Shift+V)",
    "description": "mailbox > actions. src/renderer/app.js (renderListTools); src/renderer/app.js (readerBar)"
  },
  "mailbox.actions.print": {
    "message": "Print",
    "description": "mailbox > actions. src/renderer/app.js (messageMenu)"
  },
  "mailbox.actions.read": {
    "message": "Mark as read",
    "description": "mailbox > actions. src/renderer/app.js (bulkMenu); src/renderer/app.js (messageMenu)"
  },
  "mailbox.actions.readAll": {
    "message": "Mark all as read",
    "description": "mailbox > actions. src/renderer/app.js (listAction)"
  },
  "mailbox.actions.readCount": {
    "message": {
      "one": "Marked {count} email as read",
      "other": "Marked {count} emails as read"
    },
    "description": "mailbox > actions. src/renderer/app.js (markAllRead)"
  },
  "mailbox.actions.readShortcut": {
    "message": "Mark as read (Ctrl+Q)",
    "description": "mailbox > actions. src/renderer/app.js (renderListTools)"
  },
  "mailbox.actions.star": {
    "message": "Star",
    "description": "mailbox > actions. src/renderer/app.js (renderListTools)"
  },
  "mailbox.actions.sync": {
    "message": "Sync",
    "description": "mailbox > actions. src/renderer/app.js (listAction)"
  },
  "mailbox.actions.unread": {
    "message": "Mark as unread",
    "description": "mailbox > actions. src/renderer/app.js (bulkMenu); src/renderer/app.js (messageMenu)"
  },
  "mailbox.actions.unreadShortcut": {
    "message": "Mark as unread (Ctrl+U)",
    "description": "mailbox > actions. src/renderer/app.js (renderListTools); src/renderer/app.js (readerBar)"
  },
  "mailbox.actions.unstar": {
    "message": "Remove star",
    "description": "mailbox > actions. src/renderer/app.js (itemHtml); src/renderer/app.js (bulkMenu); src/renderer/app.js (messageMenu); src/renderer/app.js (renderReader)"
  },
  "mailbox.delete.countAction": {
    "message": {
      "one": "Delete {count} email",
      "other": "Delete {count} emails"
    },
    "description": "mailbox > delete. src/renderer/app.js (bulkMenu)"
  },
  "mailbox.delete.deletedCount": {
    "message": {
      "one": "{count} email deleted",
      "other": "{count} emails deleted"
    },
    "description": "mailbox > delete. src/renderer/app.js (emptyCurrent); src/renderer/app.js (removeMessages)"
  },
  "mailbox.delete.deletedOne": {
    "message": "Deleted",
    "description": "mailbox > delete. src/renderer/app.js (removeMessages)"
  },
  "mailbox.delete.movedCount": {
    "message": {
      "one": "{count} email moved to Trash",
      "other": "{count} emails moved to Trash"
    },
    "description": "mailbox > delete. src/renderer/app.js (removeMessages)"
  },
  "mailbox.delete.movedOne": {
    "message": "Moved to Trash",
    "description": "mailbox > delete. src/renderer/app.js (removeMessages)"
  },
  "mailbox.delete.noTrash": {
    "message": "This account has no Trash folder. The email will be permanently deleted from the server.",
    "description": "mailbox > delete. src/renderer/app.js (removeMessages)"
  },
  "mailbox.delete.noTrashCount": {
    "message": {
      "one": "This account has no Trash folder. {count} email will be permanently deleted from the server.",
      "other": "This account has no Trash folder. {count} emails will be permanently deleted from the server."
    },
    "description": "mailbox > delete. src/renderer/app.js (removeMessages)"
  },
  "mailbox.delete.one": {
    "message": "This email will be permanently deleted.",
    "description": "mailbox > delete. src/renderer/app.js (removeMessages)"
  },
  "mailbox.delete.permanentCount": {
    "message": {
      "one": "{count} email will be permanently deleted.",
      "other": "{count} emails will be permanently deleted."
    },
    "description": "mailbox > delete. src/renderer/app.js (removeMessages)"
  },
  "mailbox.delete.title": {
    "message": "Permanently delete?",
    "description": "mailbox > delete. src/renderer/app.js (removeMessages)"
  },
  "mailbox.drag.messages": {
    "message": {
      "one": "{count} message",
      "other": "{count} messages"
    },
    "description": "mailbox > drag. src/renderer/app.js (bindList)"
  },
  "mailbox.drag.one": {
    "message": "1 message",
    "description": "mailbox > drag. Drag preview fallback for a single message without a subject."
  },
  "mailbox.empty.filter": {
    "message": "No emails match this filter",
    "description": "mailbox > empty. src/renderer/app.js (listHtml)"
  },
  "mailbox.empty.folder": {
    "message": "No emails",
    "description": "mailbox > empty. src/renderer/app.js (listHtml)"
  },
  "mailbox.empty.search": {
    "message": "No results",
    "description": "mailbox > empty. src/renderer/app.js (listHtml)"
  },
  "mailbox.empty.searchAll": {
    "message": "Try searching all folders.",
    "description": "mailbox > empty. src/renderer/app.js (listHtml)"
  },
  "mailbox.empty.searchAllAction": {
    "message": "Search all folders",
    "description": "mailbox > empty. src/renderer/app.js (listHtml)"
  },
  "mailbox.empty.searchOther": {
    "message": "Try another search term.",
    "description": "mailbox > empty. src/renderer/app.js (listHtml)"
  },
  "mailbox.empty.showAll": {
    "message": "Show all",
    "description": "mailbox > empty. src/renderer/app.js (listHtml)"
  },
  "mailbox.empty.syncing": {
    "message": "Syncing...",
    "description": "mailbox > empty. src/renderer/app.js (listHtml)"
  },
  "mailbox.emptyFolder.action": {
    "message": "Empty {folder}",
    "description": "mailbox > emptyFolder. src/renderer/app.js (listAction)"
  },
  "mailbox.emptyFolder.confirm": {
    "message": "Empty",
    "description": "mailbox > emptyFolder. src/renderer/app.js (emptyCurrent)"
  },
  "mailbox.emptyFolder.permanent": {
    "message": "All emails in this folder will be permanently deleted.",
    "description": "mailbox > emptyFolder. src/renderer/app.js (emptyCurrent)"
  },
  "mailbox.emptyFolder.title": {
    "message": "Empty {folder}?",
    "description": "mailbox > emptyFolder. src/renderer/app.js (emptyCurrent)"
  },
  "mailbox.emptyFolder.trash": {
    "message": "All emails in this folder will be moved to Trash.",
    "description": "mailbox > emptyFolder. src/renderer/app.js (emptyCurrent)"
  },
  "mailbox.export.action": {
    "message": "Export as .eml",
    "description": "mailbox > export. src/renderer/app.js (messageMenu)"
  },
  "mailbox.export.done": {
    "message": "Exported",
    "description": "mailbox > export. src/renderer/app.js (messageMenu)"
  },
  "mailbox.filters.all": {
    "message": "All",
    "description": "mailbox > filters. src/renderer/app.js"
  },
  "mailbox.filters.attachments": {
    "message": "Attachments",
    "description": "mailbox > filters. src/renderer/app.js; src/renderer/app.js (attachmentsHtml); src/renderer/composer.js (template)"
  },
  "mailbox.filters.label": {
    "message": "Filter",
    "description": "mailbox > filters. src/renderer/app.js (renderListTools)"
  },
  "mailbox.filters.starred": {
    "message": "Starred",
    "description": "mailbox > filters. src/renderer/app.js"
  },
  "mailbox.filters.unread": {
    "message": "Unread",
    "description": "mailbox > filters. src/renderer/app.js; src/renderer/app.js (itemHtml); src/renderer/app.js (renderReader)"
  },
  "mailbox.folders.archive": {
    "message": "Archive",
    "description": "mailbox > folders. src/renderer/app.js; src/renderer/app.js (archiveMessages); src/renderer/settings.js"
  },
  "mailbox.folders.created": {
    "message": "Folder {name} created",
    "description": "mailbox > folders. src/renderer/app.js (newFolder)"
  },
  "mailbox.folders.drafts": {
    "message": "Drafts",
    "description": "mailbox > folders. src/renderer/app.js; src/renderer/settings.js"
  },
  "mailbox.folders.inbox": {
    "message": "Inbox",
    "description": "mailbox > folders. src/renderer/app.js; src/renderer/app.js (viewLabel)"
  },
  "mailbox.folders.junk": {
    "message": "Spam",
    "description": "mailbox > folders. src/renderer/app.js; src/renderer/settings.js"
  },
  "mailbox.folders.name": {
    "message": "Folder name",
    "description": "mailbox > folders. src/renderer/app.js (newFolder)"
  },
  "mailbox.folders.new": {
    "message": "New folder",
    "description": "mailbox > folders. src/renderer/app.js (renderSidebar); src/renderer/app.js (newFolder)"
  },
  "mailbox.folders.saved": {
    "message": "Saved emails",
    "description": "mailbox > folders. src/renderer/app.js; src/renderer/settings.js"
  },
  "mailbox.folders.sent": {
    "message": "Sent",
    "description": "mailbox > folders. src/renderer/app.js; src/renderer/settings.js"
  },
  "mailbox.folders.starred": {
    "message": "Starred",
    "description": "mailbox > folders. src/renderer/app.js; src/renderer/settings.js"
  },
  "mailbox.folders.trash": {
    "message": "Trash",
    "description": "mailbox > folders. src/renderer/app.js; src/renderer/settings.js"
  },
  "mailbox.folders.vip": {
    "message": "VIPs",
    "description": "mailbox > folders. src/renderer/app.js; src/renderer/settings.js; src/renderer/settings.js (render)"
  },
  "mailbox.layout.folders": {
    "message": "Folders",
    "description": "mailbox > layout. src/renderer/app.js (renderShell); src/renderer/app.js (renderSidebar)"
  },
  "mailbox.layout.listWidth": {
    "message": "Message list width",
    "description": "mailbox > layout. src/renderer/app.js (renderShell)"
  },
  "mailbox.layout.message": {
    "message": "Message",
    "description": "mailbox > layout. src/renderer/app.js (renderShell)"
  },
  "mailbox.layout.messages": {
    "message": "Messages",
    "description": "mailbox > layout. src/renderer/app.js (renderShell)"
  },
  "mailbox.list.olderUnreadHint": {
    "message": {
      "one": "Rukoo keeps the newest {kept} messages of each inbox. {count} unread message is older than those. It stays on the server, counts toward the Inbox total and isn't in this list, so Mark all as read doesn't reach it.",
      "other": "Rukoo keeps the newest {kept} messages of each inbox. {count} unread messages are older than those. They stay on the server, count toward the Inbox total and aren't in this list, so Mark all as read doesn't reach them."
    },
    "description": "Message list header: tooltip on the inbox subtitle when unread mail is older than the messages the list holds. {kept} is how many inbox messages Rukoo keeps per account (300); {count} the older unread messages the server counted. src/renderer/app.js (listTitle)"
  },
  "mailbox.list.olderUnreadSummary": {
    "message": "{older} unread in older mail · {scope}",
    "description": "Message list header: inbox subtitle when every message in the list is read but the server counted unread mail older than it. {older} is that count, {scope} the account address or All accounts. src/renderer/app.js (listTitle)"
  },
  "mailbox.list.sentTo": {
    "message": "To: {recipients}",
    "description": "mailbox > list. src/renderer/app.js (senderLine)"
  },
  "mailbox.list.unreadSummary": {
    "message": "{count} unread · {scope}",
    "description": "mailbox > list. src/renderer/app.js (listTitle)"
  },
  "mailbox.list.unreadSummaryOlder": {
    "message": "{count} unread, {older} more in older mail · {scope}",
    "description": "Message list header: inbox subtitle when the server counted unread mail older than the messages the list holds. {count} is the unread mail in the list (what the Unread filter shows), {older} the older unread mail; together they make the sidebar's Inbox count. {scope} is the account address or All accounts. src/renderer/app.js (listTitle)"
  },
  "mailbox.message.attachment": {
    "message": "Attachment",
    "description": "mailbox > message. src/renderer/app.js (itemHtml)"
  },
  "mailbox.message.collapse": {
    "message": "Collapse",
    "description": "mailbox > message. src/renderer/app.js (itemHtml)"
  },
  "mailbox.message.noRecipient": {
    "message": "(No recipient)",
    "description": "Message list: sent email with no recipient."
  },
  "mailbox.message.noSubject": {
    "message": "(no subject)",
    "description": "mailbox > message. Message list and reader: fallback when the email has no subject."
  },
  "mailbox.message.unknownSender": {
    "message": "(Unknown sender)",
    "description": "Fallback when an email has neither a sender name nor address. Shared by the message list and reader header in src/renderer/app.js (senderLine, renderReader)."
  },
  "mailbox.message.read": {
    "message": "Read",
    "description": "mailbox > message. src/renderer/app.js (itemHtml); src/renderer/app.js (renderReader)"
  },
  "mailbox.message.replied": {
    "message": "Replied",
    "description": "mailbox > message. src/renderer/app.js (itemHtml)"
  },
  "mailbox.message.select": {
    "message": "Select",
    "description": "mailbox > message. src/renderer/app.js (itemHtml)"
  },
  "mailbox.message.stack": {
    "message": {
      "one": "{count} more email from {sender}",
      "other": "{count} more emails from {sender}"
    },
    "description": "mailbox > message. Message list: tooltip on the collapsed sender stack button."
  },
  "mailbox.message.vip": {
    "message": "VIP",
    "description": "mailbox > message. src/renderer/app.js (itemHtml); src/renderer/app.js (renderReader)"
  },
  "mailbox.message.account": {
    "message": "Account: {account}",
    "description": "mailbox > message. Screen reader label of the dot in an account's color on the sender's picture, in a list of several accounts. {account}: the account's display name or address. src/renderer/app.js (itemHtml)"
  },
  "mailbox.move.alreadyThere": {
    "message": "These emails are already there",
    "description": "mailbox > move. src/renderer/app.js (moveMessages)"
  },
  "mailbox.move.count": {
    "message": {
      "one": "{count} email moved to {folder}",
      "other": "{count} emails moved to {folder}"
    },
    "description": "mailbox > move. src/renderer/app.js (moveMessages)"
  },
  "mailbox.move.countTitle": {
    "message": {
      "one": "Move {count} email to",
      "other": "Move {count} emails to"
    },
    "description": "mailbox > move. src/renderer/app.js (pickFolder)"
  },
  "mailbox.move.noArchive": {
    "message": "No archive folder is available",
    "description": "mailbox > move. src/renderer/app.js (moveMessages)"
  },
  "mailbox.move.one": {
    "message": "Moved to {folder}",
    "description": "mailbox > move. src/renderer/app.js (moveMessages)"
  },
  "mailbox.move.title": {
    "message": "Move to",
    "description": "mailbox > move. src/renderer/app.js (pickFolder)"
  },
  "mailbox.saved.save": {
    "message": "Save to Saved emails",
    "description": "mailbox > saved. src/renderer/app.js (messageMenu)"
  },
  "mailbox.saved.saved": {
    "message": "Saved to Saved emails",
    "description": "mailbox > saved. src/renderer/app.js (messageMenu)"
  },
  "mailbox.search.account": {
    "message": "This account",
    "description": "mailbox > search. src/renderer/app.js (searchScopeLabel)"
  },
  "mailbox.search.allAccounts": {
    "message": "All accounts",
    "description": "mailbox > search. src/renderer/app.js (searchScopeLabel); src/renderer/app.js (renderSidebar); src/renderer/app.js (accountMenu); src/renderer/app.js (listTitle)"
  },
  "mailbox.search.allFolders": {
    "message": "All folders",
    "description": "mailbox > search. src/renderer/app.js (searchScopeLabel)"
  },
  "mailbox.search.clearShortcut": {
    "message": "Clear search (Esc)",
    "description": "mailbox > search. src/renderer/app.js (renderListHead)"
  },
  "mailbox.search.folder": {
    "message": "This folder",
    "description": "mailbox > search. src/renderer/app.js (searchScopeLabel)"
  },
  "mailbox.search.folderNamed": {
    "message": "This folder ({folder})",
    "description": "mailbox > search. src/renderer/app.js (renderSearch)"
  },
  "mailbox.search.in": {
    "message": "Search in",
    "description": "mailbox > search. src/renderer/app.js (renderSearch)"
  },
  "mailbox.search.label": {
    "message": "Search",
    "description": "mailbox > search. src/renderer/app.js (renderSearch)"
  },
  "mailbox.search.placeholder": {
    "message": "Search in {scope}",
    "description": "mailbox > search. src/renderer/app.js (renderSearch)"
  },
  "mailbox.search.results": {
    "message": "Search results",
    "description": "mailbox > search. src/renderer/app.js (listTitle)"
  },
  "mailbox.search.scope": {
    "message": "Search scope",
    "description": "mailbox > search. src/renderer/app.js (renderSearch)"
  },
  "mailbox.search.summary": {
    "message": {
      "one": "{count} result for \"{query}\" in {scope}",
      "other": "{count} results for \"{query}\" in {scope}"
    },
    "description": "mailbox > search. src/renderer/app.js (listTitle)"
  },
  "mailbox.selection.allShortcut": {
    "message": "Select all (Ctrl+A)",
    "description": "mailbox > selection. src/renderer/app.js (renderListTools)"
  },
  "mailbox.selection.clear": {
    "message": "Clear selection",
    "description": "mailbox > selection. src/renderer/app.js (renderReader)"
  },
  "mailbox.selection.clearShortcut": {
    "message": "Clear selection (Esc)",
    "description": "mailbox > selection. src/renderer/app.js (renderListTools)"
  },
  "mailbox.selection.count": {
    "message": "{count} selected",
    "description": "mailbox > selection. Message list selection toolbar: number of selected messages."
  },
  "mailbox.selection.none": {
    "message": "Deselect all",
    "description": "mailbox > selection. src/renderer/app.js (renderListTools)"
  },
  "mailbox.selection.summary": {
    "message": {
      "one": "{count} email selected",
      "other": "{count} emails selected"
    },
    "description": "mailbox > selection. Reading pane: heading shown when selecting multiple emails."
  },
  "mailbox.sidebar.collapse": {
    "message": "Collapse sidebar",
    "description": "mailbox > sidebar. src/renderer/app.js (renderSidebar)"
  },
  "mailbox.sidebar.expand": {
    "message": "Expand sidebar",
    "description": "mailbox > sidebar. src/renderer/app.js (renderSidebar)"
  },
  "mailbox.sort.menu": {
    "message": "Sort by...",
    "description": "mailbox > sort. src/renderer/app.js (listAction)"
  },
  "mailbox.sort.newest": {
    "message": "Date (newest first)",
    "description": "mailbox > sort. src/renderer/app.js (chooseSort)"
  },
  "mailbox.sort.oldest": {
    "message": "Date (oldest first)",
    "description": "mailbox > sort. src/renderer/app.js (chooseSort)"
  },
  "mailbox.sort.sender": {
    "message": "Sender",
    "description": "mailbox > sort. src/renderer/app.js (chooseSort)"
  },
  "mailbox.sort.title": {
    "message": "Sort by",
    "description": "mailbox > sort. src/renderer/app.js (chooseSort)"
  },
  "mailbox.sort.unread": {
    "message": "Unread first",
    "description": "mailbox > sort. src/renderer/app.js (chooseSort)"
  },
  "mailbox.spam.add": {
    "message": "Add to spam addresses",
    "description": "mailbox > spam. src/renderer/app.js (messageMenu)"
  },
  "mailbox.spam.added": {
    "message": "Added to spam addresses",
    "description": "mailbox > spam. src/renderer/app.js (messageMenu)"
  },
  "mailbox.spam.confirm": {
    "message": "Add to spam addresses?",
    "description": "mailbox > spam. src/renderer/app.js (messageMenu)"
  },
  "mailbox.spam.confirmHelp": {
    "message": "Emails from {address} will no longer appear in your Inbox.",
    "description": "mailbox > spam. src/renderer/app.js (messageMenu)"
  },
  "mailbox.sync.date": {
    "message": "Updated on {date}",
    "description": "mailbox > sync. src/renderer/app.js (syncStatus)"
  },
  "mailbox.sync.done": {
    "message": "Synced",
    "description": "mailbox > sync. src/renderer/app.js (syncNow); src/renderer/settings.js (openSettings)"
  },
  "mailbox.sync.failed": {
    "message": "Sync failed",
    "description": "mailbox > sync. src/renderer/app.js (syncStatus)"
  },
  "mailbox.sync.justNow": {
    "message": "Updated just now",
    "description": "mailbox > sync. src/renderer/app.js (syncStatus)"
  },
  "mailbox.sync.minutesAgo": {
    "message": {
      "one": "Updated {count} minute ago",
      "other": "Updated {count} minutes ago"
    },
    "description": "mailbox > sync. src/renderer/app.js (syncStatus)"
  },
  "mailbox.sync.never": {
    "message": "Not synced yet",
    "description": "mailbox > sync. src/renderer/app.js (syncStatus); src/renderer/settings.js (render)"
  },
  "mailbox.sync.shortcut": {
    "message": "Sync (F5)",
    "description": "mailbox > sync. src/renderer/app.js (renderListHead)"
  },
  "mailbox.sync.syncing": {
    "message": "Syncing...",
    "description": "mailbox > sync. src/renderer/app.js (syncStatus); src/renderer/app.js (syncNow); src/renderer/settings.js (render); src/renderer/settings.js (openSettings)"
  },
  "mailbox.sync.time": {
    "message": "Updated at {time}",
    "description": "mailbox > sync. src/renderer/app.js (syncStatus)"
  },
  "mailbox.sync.tooltip": {
    "message": "Updated on {date} at {time}. Click to sync (F5).",
    "description": "mailbox > sync. src/renderer/app.js (syncStatus)"
  },
  "mailbox.undo.restored": {
    "message": "Restored",
    "description": "mailbox > undo. src/renderer/app.js (undoMoves)"
  },
  "mailbox.undo.restoredCount": {
    "message": {
      "one": "{count} email restored",
      "other": "{count} emails restored"
    },
    "description": "mailbox > undo. src/renderer/app.js (undoMoves)"
  },
  "mailbox.vip.add": {
    "message": "Add to VIPs",
    "description": "mailbox > vip. src/renderer/app.js (messageMenu); src/renderer/app.js (personMenu)"
  },
  "mailbox.vip.added": {
    "message": "Added to VIPs",
    "description": "mailbox > vip. src/renderer/app.js (personMenu)"
  },
  "mailbox.vip.addedSender": {
    "message": "{sender} added to VIPs",
    "description": "mailbox > vip. src/renderer/app.js (messageMenu)"
  },
  "mailbox.vip.remove": {
    "message": "Remove from VIPs",
    "description": "mailbox > vip. src/renderer/app.js (messageMenu); src/renderer/app.js (personMenu)"
  },
  "mailbox.vip.removed": {
    "message": "Removed from VIPs",
    "description": "mailbox > vip. src/renderer/app.js (personMenu)"
  },
  "mailbox.vip.removedSender": {
    "message": "{sender} removed from VIPs",
    "description": "mailbox > vip. src/renderer/app.js (messageMenu)"
  },
  "native.attachments.defaultName": {
    "message": "attachment",
    "description": "Default filename for an attachment without a filename."
  },
  "native.image.defaultName": {
    "message": "image",
    "description": "File name, without extension, for an image from an email whose address has no name, when it is saved or opened. Lower case, like a file name. src/main/mailimage.js"
  },
  "native.attachments.defaultNumbered": {
    "message": "attachment-{number}",
    "description": "Default attachment filename with its one-based index."
  },
  "native.attachments.directory": {
    "message": "Save attachments to",
    "description": "native > attachments. src/main/main.js (saveAllAttachments)"
  },
  "native.attachments.images": {
    "message": "Images",
    "description": "native > attachments. src/main/main.js"
  },
  "native.attachments.open": {
    "message": "Open attachment",
    "description": "native > attachments. src/main/main.js"
  },
  "native.attachments.riskyHelp": {
    "message": "This file type cannot be opened from email. Save it and only open it if you trust the sender.",
    "description": "native > attachments. src/main/main.js"
  },
  "native.attachments.riskyTitle": {
    "message": "{filename} can launch a program.",
    "description": "native > attachments. src/main/main.js"
  },
  "native.attachments.save": {
    "message": "Save...",
    "description": "Save action in native attachment file dialogs in src/main/main.js. This action is separate from the composer draft-saving progress status."
  },
  "native.export.defaultName": {
    "message": "message",
    "description": "Default filename stem for exporting an email without a subject."
  },
  "native.notifications.newCount": {
    "message": {
      "one": "{count} new email",
      "other": "{count} new emails"
    },
    "description": "native > notifications. src/main/main.js (notify); src/main/main.js"
  },
  "native.notifications.newOne": {
    "message": "1 new email",
    "description": "native > notifications. Windows taskbar badge: singular unread/new email description."
  },
  "native.tray.compose": {
    "message": "New message",
    "description": "native > tray. Menu of the Rukoo icon in the Windows notification area: opens a compose window for a new email. src/main/tray.js (TrayIcon.relabel)"
  },
  "native.tray.open": {
    "message": "Open Rukoo",
    "description": "native > tray. Menu of the Rukoo icon in the Windows notification area: shows the main window again. src/main/tray.js (TrayIcon.relabel)"
  },
  "native.tray.quit": {
    "message": "Quit",
    "description": "native > tray. Menu of the Rukoo icon in the Windows notification area: quits Rukoo, which stops sync and new-mail notifications. src/main/tray.js (TrayIcon.relabel)"
  },
  "native.tray.sync": {
    "message": "Sync now",
    "description": "native > tray. Menu of the Rukoo icon in the Windows notification area: syncs every account now. src/main/tray.js (TrayIcon.relabel)"
  },
  "native.tray.tooltip": {
    "message": {
      "one": "Rukoo Mail: {count} unread email",
      "other": "Rukoo Mail: {count} unread emails"
    },
    "description": "native > tray. Tooltip of the Rukoo icon in the Windows notification area while there is unread mail; count is the unread mail in the inboxes of all accounts. Without unread mail the tooltip is just the product name. src/main/tray.js (tooltip)"
  },
  "reader.actions.edit": {
    "message": "Edit",
    "description": "reader > actions. src/renderer/app.js (readerBar)"
  },
  "reader.actions.editDraftShortcut": {
    "message": "Edit draft (Enter)",
    "description": "reader > actions. src/renderer/app.js (readerBar)"
  },
  "reader.actions.forwardShortcut": {
    "message": "Forward (Ctrl+F)",
    "description": "reader > actions. src/renderer/app.js (readerBar)"
  },
  "reader.actions.label": {
    "message": "Actions",
    "description": "reader > actions. src/renderer/app.js (readerBar)"
  },
  "reader.actions.more": {
    "message": "More actions",
    "description": "reader > actions. src/renderer/app.js (readerBar), and the button that opens the menu for checked emails in the list toolbar (renderListTools)"
  },
  "reader.actions.replyAllShortcut": {
    "message": "Reply all (Ctrl+Shift+R)",
    "description": "reader > actions. src/renderer/app.js (readerBar)"
  },
  "reader.actions.replyShortcut": {
    "message": "Reply (Ctrl+R)",
    "description": "reader > actions. src/renderer/app.js (readerBar)"
  },
  "reader.address.copied": {
    "message": "Copied",
    "description": "reader > address. src/renderer/app.js (personMenu)"
  },
  "reader.address.copy": {
    "message": "Copy email address",
    "description": "reader > address. src/renderer/app.js (personMenu)"
  },
  "reader.attachments.count": {
    "message": {
      "one": "{count} attachment",
      "other": "{count} attachments"
    },
    "description": "Reading pane attachment section toggle in src/renderer/app.js (attachmentsHtml). count is the number of attached files; includes plural forms."
  },
  "reader.attachments.open": {
    "message": "Open: {filename}",
    "description": "Tooltip on image and file attachment open buttons in src/renderer/app.js (attachmentsHtml). filename is the attachment's original filename."
  },
  "reader.attachments.saveAll": {
    "message": "Save all",
    "description": "reader > attachments. src/renderer/app.js (attachmentsHtml)"
  },
  "reader.attachments.savedCount": {
    "message": {
      "one": "{count} attachment saved",
      "other": "{count} attachments saved"
    },
    "description": "reader > attachments. src/renderer/app.js (bindReader)"
  },
  "reader.attachments.savedOne": {
    "message": "Attachment saved",
    "description": "reader > attachments. src/renderer/app.js (bindReader)"
  },
  "reader.content.label": {
    "message": "Email content",
    "description": "reader > content. src/renderer/app.js (renderReader)"
  },
  "reader.content.openLink": {
    "message": "Open link",
    "description": "Menu item: opens the link in the default browser. Context menu on an email's content (right-click in the reader or the quoted email in the composer). src/renderer/mailframe.js (frameMenu)"
  },
  "reader.content.copyLink": {
    "message": "Copy link address",
    "description": "Menu item: copies the link's web address. Context menu on an email's content (right-click in the reader or the quoted email in the composer). src/renderer/mailframe.js (frameMenu)"
  },
  "reader.content.linkCopied": {
    "message": "Link copied",
    "description": "Toast after Copy link address. Context menu on an email's content (right-click in the reader or the quoted email in the composer). src/renderer/mailframe.js (frameMenu)"
  },
  "reader.content.openImage": {
    "message": "Open image",
    "description": "Menu item: opens the image in the computer's default image viewer. Context menu on an email's content (right-click in the reader or the quoted email in the composer). src/renderer/mailframe.js (frameMenu)"
  },
  "reader.content.copyImage": {
    "message": "Copy image",
    "description": "Menu item: copies the image itself to the clipboard. Context menu on an email's content (right-click in the reader or the quoted email in the composer). src/renderer/mailframe.js (frameMenu)"
  },
  "reader.content.imageCopied": {
    "message": "Image copied",
    "description": "Toast after Copy image. Context menu on an email's content (right-click in the reader or the quoted email in the composer). src/renderer/mailframe.js (frameMenu)"
  },
  "reader.content.saveImage": {
    "message": "Save image as…",
    "description": "Menu item: asks where to save the image, then saves it. Context menu on an email's content (right-click in the reader or the quoted email in the composer). src/renderer/mailframe.js (frameMenu)"
  },
  "reader.content.imageSaved": {
    "message": "Image saved",
    "description": "Toast after the image was saved. Context menu on an email's content (right-click in the reader or the quoted email in the composer). src/renderer/mailframe.js (frameMenu)"
  },
  "reader.content.copy": {
    "message": "Copy",
    "description": "Menu item: copies the selected text. Context menu on an email's content (right-click in the reader or the quoted email in the composer). src/renderer/mailframe.js (frameMenu)"
  },
  "reader.content.selectAll": {
    "message": "Select all",
    "description": "Menu item: selects all of the email's text. Context menu on an email's content (right-click in the reader or the quoted email in the composer). src/renderer/mailframe.js (frameMenu)"
  },
  "reader.details.hide": {
    "message": "Hide all details",
    "description": "Reading pane recipient summary tooltip when full headers are open."
  },
  "reader.details.show": {
    "message": "Show all details",
    "description": "Reading pane recipient summary tooltip when full headers are closed."
  },
  "reader.empty.browse": {
    "message": "Browse",
    "description": "reader > empty. Shared interface label."
  },
  "reader.empty.title": {
    "message": "No email selected",
    "description": "reader > empty. src/renderer/app.js (renderReader)"
  },
  "reader.headers.bcc": {
    "message": "Bcc",
    "description": "reader > headers. src/renderer/app.js (detailsHtml); src/renderer/composer.js (template); src/renderer/composer.js (mountComposer)"
  },
  "reader.headers.cc": {
    "message": "Cc",
    "description": "reader > headers. src/renderer/app.js (detailsHtml); src/renderer/composer.js (template); src/renderer/composer.js (mountComposer)"
  },
  "reader.headers.date": {
    "message": "Date",
    "description": "reader > headers. src/renderer/app.js (detailsHtml)"
  },
  "reader.headers.from": {
    "message": "From",
    "description": "reader > headers. src/renderer/app.js (detailsHtml); src/renderer/composer.js (template)"
  },
  "reader.headers.replyTo": {
    "message": "Reply to",
    "description": "reader > headers. src/renderer/app.js (detailsHtml)"
  },
  "reader.headers.subject": {
    "message": "Subject",
    "description": "reader > headers. src/renderer/composer.js (template)"
  },
  "reader.headers.to": {
    "message": "To",
    "description": "reader > headers. src/renderer/app.js (detailsHtml); src/renderer/composer.js (template); src/renderer/composer.js (mountComposer)"
  },
  "reader.navigation.back": {
    "message": "Back to message list",
    "description": "reader > navigation. src/renderer/app.js (readerBar)"
  },
  "reader.navigation.expand": {
    "message": "Expand reading pane",
    "description": "reader > navigation. src/renderer/app.js (readerBar)"
  },
  "reader.navigation.next": {
    "message": "Next (Down arrow)",
    "description": "reader > navigation. src/renderer/app.js (readerBar)"
  },
  "reader.navigation.previous": {
    "message": "Previous (Up arrow)",
    "description": "reader > navigation. src/renderer/app.js (readerBar)"
  },
  "reader.navigation.showList": {
    "message": "Show message list (Esc)",
    "description": "reader > navigation. src/renderer/app.js (readerBar)"
  },
  "reader.recipients.me": {
    "message": "me",
    "description": "reader > recipients. Reading pane: replaces a recipient address belonging to the current account."
  },
  "reader.recipients.others": {
    "message": {
      "one": " and {count} other",
      "other": " and {count} others"
    },
    "description": "reader > recipients. src/renderer/app.js (recipientsHtml)"
  },
  "reader.recipients.summary": {
    "message": "to {recipients}",
    "description": "reader > recipients. src/renderer/app.js (recipientsHtml)"
  },
  "reader.recipients.unknown": {
    "message": "to unknown recipients",
    "description": "Reading pane recipient summary when the email has no recipients."
  },
  "reader.related.count": {
    "message": {
      "one": "{count} related email",
      "other": "{count} related emails"
    },
    "description": "Reading pane: the toggle above the list of related emails under the open email's header: the rest of its conversation in the synced folders, and mail with the same subject and the same people. {count}: how many. src/renderer/app.js (relatedHtml)"
  },
  "reader.related.label": {
    "message": "Related emails",
    "description": "Reading pane: screen reader name of the section that lists related emails under the open email's header. src/renderer/app.js (relatedHtml)"
  },
  "reader.related.you": {
    "message": "You",
    "description": "Reading pane: the sender of a related email the user sent, in the list of related emails. src/renderer/app.js (relatedHtml)"
  },
  "reader.related.showAll": {
    "message": "Show all {count}",
    "description": "Reading pane: link under the first few related emails that shows the rest. {count}: how many related emails there are in all, always more than five. src/renderer/app.js (relatedHtml)"
  },
  "reader.unsubscribe.action": {
    "message": "Unsubscribe",
    "description": "reader > unsubscribe. src/renderer/app.js (renderReader); src/renderer/app.js (unsubscribe)"
  },
  "reader.unsubscribe.done": {
    "message": "Unsubscribed from {sender}",
    "description": "reader > unsubscribe. src/renderer/app.js (unsubscribe)"
  },
  "reader.unsubscribe.linkHelp": {
    "message": "Rukoo Mail will use the sender's unsubscribe link.",
    "description": "reader > unsubscribe. src/renderer/app.js (unsubscribe)"
  },
  "reader.unsubscribe.mailHelp": {
    "message": "Rukoo Mail will prepare an unsubscribe email for you.",
    "description": "reader > unsubscribe. src/renderer/app.js (unsubscribe)"
  },
  "reader.unsubscribe.opened": {
    "message": "The unsubscribe page has opened in your browser",
    "description": "reader > unsubscribe. src/renderer/app.js (unsubscribe)"
  },
  "reader.unsubscribe.subject": {
    "message": "Unsubscribe",
    "description": "reader > unsubscribe. src/renderer/app.js (unsubscribe)"
  },
  "reader.unsubscribe.title": {
    "message": "Unsubscribe from {sender}?",
    "description": "reader > unsubscribe. src/renderer/app.js (unsubscribe)"
  },
  "reader.unsubscribe.tooltip": {
    "message": "Unsubscribe from these emails",
    "description": "reader > unsubscribe. src/renderer/app.js (renderReader)"
  },
  "settings.about.description": {
    "message": "An email client for Windows, macOS and Linux, built with Electron. Data is stored in:",
    "description": "settings > about. src/renderer/settings.js (openSettings)"
  },
  "settings.about.help": {
    "message": "Version and storage location",
    "description": "settings > about. src/renderer/settings.js (render)"
  },
  "settings.about.title": {
    "message": "About Rukoo Mail",
    "description": "settings > about. src/renderer/settings.js (render)"
  },
  "settings.about.version": {
    "message": "Version {version}",
    "description": "settings > about. About dialog: application version."
  },
  "settings.account.color": {
    "message": "Account color",
    "description": "settings > account. src/renderer/settings.js (render); src/renderer/settings.js (openSettings)"
  },
  "settings.account.defaultBadge": {
    "message": "Default",
    "description": "settings > accounts. Small badge after the default account's address in the accounts overview, and after the default address on the Sender addresses page. src/renderer/settings.js (defaultBadge)"
  },
  "settings.account.from": {
    "message": "Send as",
    "description": "settings > account. Row and dialog title for the address this account sends from when it has aliases; not the default account. src/renderer/settings.js (render); src/renderer/settings.js (openSettings)"
  },
  "settings.account.googleReauth": {
    "message": "Sign in to Google again",
    "description": "settings > account. src/renderer/settings.js (render)"
  },
  "settings.account.googleReauthHelp": {
    "message": "Use this if Google has revoked access.",
    "description": "settings > account. src/renderer/settings.js (render)"
  },
  "settings.account.googleSwitch": {
    "message": "Switch to Google sign-in",
    "description": "settings > account. src/renderer/settings.js (render)"
  },
  "settings.account.googleSwitchHelp": {
    "message": "Sign in through your browser instead of using an app password.",
    "description": "settings > account. src/renderer/settings.js (render)"
  },
  "settings.account.identities": {
    "message": "Sender addresses",
    "description": "settings > account. src/renderer/settings.js (render)"
  },
  "settings.account.isDefault": {
    "message": "Default account",
    "description": "settings > account. First row on the default account's page, above settings.account.makeDefaultHelp; it states a fact and has no action. src/renderer/settings.js (render)"
  },
  "settings.account.lastSync": {
    "message": "Last synced on {date} at {time}",
    "description": "settings > account. src/renderer/settings.js (render)"
  },
  "settings.account.makeDefault": {
    "message": "Set as default account",
    "description": "settings > account. src/renderer/settings.js (render)"
  },
  "settings.account.makeDefaultHelp": {
    "message": "New emails will be sent from this account.",
    "description": "settings > account. Under settings.account.makeDefault on other accounts' pages, and under settings.account.isDefault on the default account's page. src/renderer/settings.js (render)"
  },
  "settings.account.name": {
    "message": "Display name",
    "description": "settings > account. src/renderer/settings.js (render); src/renderer/settings.js (openSettings)"
  },
  "settings.account.noAliases": {
    "message": "Account address only",
    "description": "settings > account. src/renderer/settings.js (render)"
  },
  "settings.account.remove": {
    "message": "Remove account",
    "description": "settings > account. src/renderer/settings.js (render)"
  },
  "settings.account.removed": {
    "message": "Account removed",
    "description": "settings > account. src/renderer/settings.js (openSettings)"
  },
  "settings.account.removeHelp": {
    "message": "{email} and all locally stored emails for this account will be removed from this PC. Everything on the server will be kept.",
    "description": "settings > account. src/renderer/settings.js (openSettings)"
  },
  "settings.account.removeTitle": {
    "message": "Remove account?",
    "description": "settings > account. src/renderer/settings.js (openSettings)"
  },
  "settings.account.server": {
    "message": "Server settings",
    "description": "settings > account. src/renderer/settings.js (render)"
  },
  "settings.account.sync": {
    "message": "Sync now",
    "description": "settings > account. src/renderer/settings.js (render)"
  },
  "settings.accounts.add": {
    "message": "Add account",
    "description": "settings > accounts. src/renderer/app.js (accountMenu); src/renderer/settings.js (render)"
  },
  "settings.accounts.makeDefault": {
    "message": "Set as default",
    "description": "settings > accounts. Small button beside each account that is not the default in the accounts overview; new emails then start from that account. src/renderer/settings.js (render)"
  },
  "settings.accounts.overview": {
    "message": "Accounts and settings",
    "description": "settings > accounts. src/renderer/app.js (accountMenu)"
  },
  "settings.addresses.removeHint": {
    "message": "Click to remove",
    "description": "settings > addresses. src/renderer/settings.js (render)"
  },
  "settings.agents.access": {
    "message": "Access",
    "description": "Settings, Agents page: row that sets what the agent may do without asking."
  },
  "settings.agents.accessAsk": {
    "message": "Ask before actions",
    "description": "Settings, Agents page: access choice where commands wait for approval."
  },
  "settings.agents.accessAskHint": {
    "message": "Commands and file changes wait for your approval in the chat",
    "description": "Settings, Agents page: explanation of the ask-first access choice."
  },
  "settings.agents.accessFull": {
    "message": "Full access",
    "description": "Settings, Agents page: access choice where the agent runs commands without asking."
  },
  "settings.agents.accessFullHint": {
    "message": "Runs commands without asking. Only for agents you trust completely.",
    "description": "Settings, Agents page: warning under the full access choice."
  },
  "settings.agents.permission": {
    "message": "Permission level",
    "description": "Settings, Agents page: row that sets which actions of agents wait for your approval, as the default for chats. Opens a choice of Ask, Auto and Full permission."
  },
  "settings.agents.permissionHelp": {
    "message": "What agents may do without asking you first. A chat can pick its own level with the shield beside Send. Commands and file changes of Claude Code and Codex follow their own Access below.",
    "description": "Settings, Agents page: explanation under the Permission level row. Access is the existing per-agent setting of Claude Code and Codex further down the page (settings.agents.access)."
  },
  "settings.agents.permissionAskHint": {
    "message": "Mail actions, calendar events and actions in other systems wait for your approval. Drafts go into the composer, where you check and send them yourself.",
    "description": "Settings, Agents page: explanation of the Ask choice in the Permission level dialog."
  },
  "settings.agents.permissionAutoHint": {
    "message": "Archiving, moving, marking and starring mail, and adding events without guests to your own calendars, happen without asking. Deleting, moving mail to Spam, unsubscribing, invitations and actions in other systems still wait for you.",
    "description": "Settings, Agents page: explanation of the Auto choice in the Permission level dialog."
  },
  "settings.agents.permissionFullHint": {
    "message": "Nothing waits for your approval. Agents can send messages and calendar invitations, delete mail and change other systems without asking. Only for agents you trust completely.",
    "description": "Settings, Agents page: warning under the Full permission choice in the Permission level dialog. It must say plainly that sending mail and invitations happens without asking."
  },
  "settings.agents.configDir": {
    "message": "Claude config folder (optional)",
    "description": "Settings, Agents page: field for the Claude Code configuration folder."
  },
  "settings.agents.configDirHint": {
    "message": "The default folder",
    "description": "Settings, Agents page: placeholder of the Claude config folder field."
  },
  "settings.agents.copied": {
    "message": "Copied. Paste it into ~/.hermes/config.yaml on {name}'s machine.",
    "description": "Settings, Agents page: toast after copying the Hermes setup."
  },
  "settings.agents.copySetup": {
    "message": "Copy Hermes setup",
    "description": "Settings, Agents page: row that copies the config block for Hermes."
  },
  "settings.agents.copySetupHelp": {
    "message": "The block for {name}'s config.yaml. It's the same for all your devices.",
    "description": "Settings, Agents page: description of the copy setup row."
  },
  "settings.agents.copyOpenClaw": {
    "message": "Copy OpenClaw setup",
    "description": "Settings, Agents page, OpenClaw card: row that copies the command that adds Rukoo's MCP server to OpenClaw."
  },
  "settings.agents.copyOpenClawHelp": {
    "message": "A command for this computer that lets OpenClaw use Rukoo's tools. It holds a token, so don't share it.",
    "description": "Settings, Agents page, OpenClaw card: description of the copy setup row for a gateway on this computer. The copied command (openclaw mcp set) contains Rukoo's MCP token for OpenClaw."
  },
  "settings.agents.copiedOpenClaw": {
    "message": "Copied. Run it in a terminal on this computer, then restart the OpenClaw gateway.",
    "description": "Settings, Agents page: toast after copying the OpenClaw setup command."
  },
  "settings.agents.gatewayUrl": {
    "message": "Gateway URL",
    "description": "Settings, Agents page, OpenClaw card: field for the address of the OpenClaw gateway."
  },
  "settings.agents.openclawAgent": {
    "message": "Agent ID (optional)",
    "description": "Settings, Agents page, OpenClaw card: field for the id of the OpenClaw agent a new chat goes to."
  },
  "settings.agents.openclawAgentHint": {
    "message": "Empty for the gateway's default agent",
    "description": "Settings, Agents page, OpenClaw card: placeholder of the agent id field."
  },
  "settings.agents.token": {
    "message": "Gateway token",
    "description": "Settings, Agents page, OpenClaw card: label of the gateway token (or password) field."
  },
  "settings.agents.tokenFirst": {
    "message": "Save the gateway token first.",
    "description": "Settings, Agents page: toast when the OpenClaw setup is copied before a gateway token is saved."
  },
  "settings.agents.tokenHide": {
    "message": "Hide token",
    "description": "Settings, Agents page: tooltip of the eye button while the OpenClaw gateway token is visible."
  },
  "settings.agents.tokenPlaceholder": {
    "message": "Paste the gateway token or password",
    "description": "Settings, Agents page: placeholder of the OpenClaw gateway token field."
  },
  "settings.agents.tokenSave": {
    "message": "Save token",
    "description": "Settings, Agents page: button that stores the OpenClaw gateway token."
  },
  "settings.agents.tokenSaved": {
    "message": "Token saved",
    "description": "Settings, Agents page: shown instead of the OpenClaw gateway token once it is stored."
  },
  "settings.agents.tokenShow": {
    "message": "Show token",
    "description": "Settings, Agents page: tooltip of the eye button on the OpenClaw gateway token field."
  },
  "settings.agents.tokenStored": {
    "message": "Token saved. Rukoo keeps it encrypted on this computer.",
    "description": "Settings, Agents page: toast after the OpenClaw gateway token was stored."
  },
  "settings.agents.copyOpenClawRemoteHelp": {
    "message": "A command for the gateway's computer, with Rukoo's Tailscale address. It holds a token, so don't share it.",
    "description": "Settings, Agents page, OpenClaw card: description of the copy setup row while a gateway on another computer may use Rukoo. The copied command (openclaw mcp set) contains Rukoo's Tailscale address and its MCP token for OpenClaw."
  },
  "settings.agents.copiedOpenClawRemote": {
    "message": "Copied with {url}. Run it on the gateway's computer, then restart the OpenClaw gateway.",
    "description": "Settings, Agents page: toast after copying the OpenClaw setup for a gateway on another computer. url is Rukoo's MCP address on Tailscale, such as http://100.64.0.5:47800/mcp."
  },
  "settings.agents.openclawNoListener": {
    "message": "Rukoo isn't listening on a Tailscale address. Check that Tailscale runs on this computer.",
    "description": "Settings, Agents page: toast when the OpenClaw setup for another computer is copied while Rukoo has no Tailscale listener."
  },
  "settings.agents.openclawRemote": {
    "message": "Let OpenClaw use Rukoo from another computer",
    "description": "Settings, Agents page, OpenClaw card: toggle that lets an OpenClaw gateway on another computer in the user's tailnet call Rukoo's tools over Tailscale."
  },
  "settings.agents.openclawRemoteHelp": {
    "message": "For a gateway elsewhere in your tailnet. Rukoo then also listens on its Tailscale address.",
    "description": "Settings, Agents page, OpenClaw card: description under the remote toggle while it is off."
  },
  "settings.agents.openclawRemoteNeedsToken": {
    "message": "OpenClaw signs in with a token Rukoo makes from the gateway token above. Enter it first.",
    "description": "Settings, Agents page, OpenClaw card: note under the remote toggle when it is on but no gateway token is saved."
  },
  "settings.phone.title": {
    "message": "Phone",
    "description": "Settings, Agents page: heading of the group that lets Rukoo on a phone use this computer."
  },
  "settings.phone.host": {
    "message": "Let my phone use this computer",
    "description": "Settings, Agents page, Phone group: toggle that lets paired phones run Claude Code and Codex chats on this computer over Tailscale."
  },
  "settings.phone.hostHelp": {
    "message": "Your phone runs Claude Code and Codex chats here over Tailscale. Rukoo then also listens on its Tailscale address.",
    "description": "Settings, Agents page, Phone group: description under the toggle while it is off."
  },
  "settings.phone.noListener": {
    "message": "Rukoo isn't listening on a Tailscale address. Check that Tailscale runs on this computer.",
    "description": "Settings, Agents page, Phone group: toast when the user starts pairing but Rukoo has no Tailscale listener to show."
  },
  "settings.phone.pair": {
    "message": "Pair a phone",
    "description": "Settings, Agents page, Phone group: row that starts pairing a phone; also the title of the dialog that asks what the phone may receive."
  },
  "settings.phone.pairHelp": {
    "message": "Shows a code to enter in Rukoo on your phone",
    "description": "Settings, Agents page, Phone group: description under the Pair a phone row."
  },
  "settings.phone.pairWithExport": {
    "message": "Pair and send my accounts and agent settings",
    "description": "Pairing dialog: choice that pairs the phone and lets it fetch the accounts and agent settings once."
  },
  "settings.phone.pairWithExportHint": {
    "message": "Includes passwords, Google sign-ins and agent keys",
    "description": "Pairing dialog and phone menu: note under the choice that sends the accounts, saying that secrets go along."
  },
  "settings.phone.pairOnly": {
    "message": "Pair only",
    "description": "Pairing dialog: choice that pairs the phone without sending accounts or settings."
  },
  "settings.phone.pairOnlyHint": {
    "message": "Accounts and keys stay on this computer",
    "description": "Pairing dialog: note under the Pair only choice."
  },
  "settings.phone.pairTitle": {
    "message": "Pair a phone",
    "description": "Title of the dialog that shows the pairing code and this computer's address."
  },
  "settings.phone.pairSteps": {
    "message": "Open Rukoo on your phone, choose to pair with a computer, and enter this address and code.",
    "description": "Pairing code dialog: what the user does on the phone."
  },
  "settings.phone.address": {
    "message": "Address",
    "description": "Pairing code dialog: label of this computer's address, such as http://100.64.0.5:47800."
  },
  "settings.phone.code": {
    "message": "Code",
    "description": "Pairing code dialog: label of the 8-character pairing code."
  },
  "settings.phone.pairExportNote": {
    "message": "Your phone can then fetch your accounts and agent settings once, in the next 10 minutes.",
    "description": "Pairing code dialog: note shown when the user chose to send the accounts and agent settings."
  },
  "settings.phone.pairExpires": {
    "message": { "one": "The code works for {count} more minute.", "other": "The code works for {count} more minutes." },
    "description": "Pairing code dialog: how long the code still works; count is whole minutes, rounded up."
  },
  "settings.phone.pairExpired": {
    "message": "The code has expired. Start again.",
    "description": "Pairing code dialog: shown in place of the code once it has expired."
  },
  "settings.phone.pairBurned": {
    "message": "The code was entered wrong too often. Start again.",
    "description": "Pairing code dialog: shown in place of the code after five wrong tries on the phone."
  },
  "settings.phone.paired": {
    "message": "Paired with {name}",
    "description": "Toast after a phone paired; name is the phone's name, such as Galaxy Z Fold7."
  },
  "settings.phone.lastSeen": {
    "message": "Last seen {date} {time}",
    "description": "Settings, Agents page, Phone group: when a paired phone last reached this computer; date and time are formatted."
  },
  "settings.phone.neverSeen": {
    "message": "Not connected yet",
    "description": "Settings, Agents page, Phone group: shown for a paired phone that has not reached this computer yet."
  },
  "settings.phone.exportPending": {
    "message": "Can fetch your accounts and settings now",
    "description": "Settings, Agents page, Phone group: shown under a paired phone while it may fetch the accounts and settings."
  },
  "settings.phone.exportAgain": {
    "message": "Send accounts and settings again",
    "description": "Phone menu: choice that lets the phone fetch the accounts and agent settings once more in the next 10 minutes."
  },
  "settings.phone.exportOpen": {
    "message": "Your phone can fetch them in the next 10 minutes",
    "description": "Toast after Send accounts and settings again."
  },
  "settings.phone.exported": {
    "message": "Sent your accounts and settings to {name}",
    "description": "Toast when a paired phone fetched the accounts and settings; name is the phone's name."
  },
  "settings.phone.remove": {
    "message": "Remove this phone",
    "description": "Phone menu: choice that unpairs the phone; also the title of the confirmation."
  },
  "settings.phone.removeConfirm": {
    "message": "{name} can no longer use this computer. Accounts it already has stay on the phone.",
    "description": "Confirmation before unpairing a phone; name is the phone's name."
  },
  "settings.phone.removeOk": {
    "message": "Remove",
    "description": "Button in the confirmation that unpairs a phone."
  },
  "settings.phone.removed": {
    "message": "Removed {name}",
    "description": "Toast after a phone was unpaired; name is the phone's name."
  },
  "settings.agents.default": {
    "message": "Default agent",
    "description": "Settings, Agents page: which agent a new chat goes to."
  },
  "settings.agents.enabled": {
    "message": "Use {name}",
    "description": "Settings, Agents page: toggle that turns an agent on or off; name is the agent."
  },
  "settings.agents.exe": {
    "message": "Program path",
    "description": "Settings, Agents page: field for the Claude Code or Codex program."
  },
  "settings.agents.exeHint": {
    "message": "Found automatically ({exe})",
    "description": "Settings, Agents page: placeholder of the program path field."
  },
  "settings.agents.help": {
    "message": "Agents keep their own tools and memory. Rukoo only adds this chat and lets them read mail and write drafts here. The model you set here is where a new chat starts; under the chat input each chat can pick its own model and effort.",
    "description": "Settings, Agents page: short explanation at the top; the chat input's model and effort menus are in agent/panel.js."
  },
  "settings.agents.key": {
    "message": "API key",
    "description": "Settings, Agents page: label of the Hermes API key field."
  },
  "settings.agents.keyHide": {
    "message": "Hide key",
    "description": "Settings, Agents page: tooltip of the eye button while the key is visible."
  },
  "settings.agents.keyPlaceholder": {
    "message": "Paste the API server key",
    "description": "Settings, Agents page: placeholder of the API key field."
  },
  "settings.agents.keyReplace": {
    "message": "Replace",
    "description": "Settings, Agents page: link to enter a new API key."
  },
  "settings.agents.keySave": {
    "message": "Save key",
    "description": "Settings, Agents page: button that stores the API key."
  },
  "settings.agents.keySaved": {
    "message": "Key saved",
    "description": "Settings, Agents page: shown instead of the key once it is stored."
  },
  "settings.agents.keyShow": {
    "message": "Show key",
    "description": "Settings, Agents page: tooltip of the eye button on the key field."
  },
  "settings.agents.keyStored": {
    "message": "Key saved. Rukoo keeps it encrypted on this computer.",
    "description": "Settings, Agents page: toast after the API key was stored."
  },
  "settings.agents.model": {
    "message": "Default model (optional)",
    "description": "Settings, Agents page: field on each agent card for the model a chat uses until it picks its own in the chat panel (agentsHtml in settings.js)."
  },
  "settings.agents.modelHint": {
    "message": "Empty for the default, for example {example}",
    "description": "Settings, Agents page: placeholder of the Claude and OpenClaw model fields; example is a model name."
  },
  "settings.agents.name": {
    "message": "Name",
    "description": "Settings, Agents page: field for the name of the Hermes agent."
  },
  "settings.agents.noAddress": {
    "message": "No Tailscale address found on this computer",
    "description": "Settings, Agents page: shown under the remote toggle when Tailscale is not running."
  },
  "settings.agents.optional": {
    "message": "Empty for the default",
    "description": "Settings, Agents page: placeholder of an optional field, such as the Hermes and Codex default model."
  },
  "settings.agents.remote": {
    "message": "Let {name} use Rukoo",
    "description": "Settings, Agents page: toggle that lets the remote Hermes agent call Rukoo over Tailscale."
  },
  "settings.agents.remoteNeedsKey": {
    "message": "{name} signs in with the API key above. Enter it first.",
    "description": "Settings, Agents page: note under the remote access toggle when it is on but no Hermes API key is saved; name is the agent."
  },
  "settings.agents.rowDesc": {
    "message": "Default: {name} · {status}",
    "description": "Settings overview row description: default agent and its status."
  },
  "settings.agents.rowTitle": {
    "message": "{name}, Claude Code, Codex and OpenClaw",
    "description": "Settings overview row that opens the agents page; name is the Hermes agent name."
  },
  "settings.agents.saved": {
    "message": "Saved",
    "description": "Settings, Agents page: short toast after a field was saved."
  },
  "settings.agents.test": {
    "message": "Test",
    "description": "Settings, Agents page: button that checks whether Claude Code or Codex can be started."
  },
  "settings.agents.testConnection": {
    "message": "Test connection",
    "description": "Settings, Agents page: button that checks the connection to Hermes or the OpenClaw gateway."
  },
  "settings.agents.testing": {
    "message": "Testing...",
    "description": "Settings, Agents page: status line while a test runs."
  },
  "settings.agents.title": {
    "message": "Agents",
    "description": "Settings: title of the page where you set up Hermes, Claude Code, Codex and OpenClaw."
  },
  "settings.agents.unavailable": {
    "message": "Agents aren't available in this version.",
    "description": "Settings: shown when this build has no agent support."
  },
  "settings.agents.url": {
    "message": "Server URL",
    "description": "Settings, Agents page: field for the Hermes API server address."
  },
  "settings.agents.shortcuts": {
    "message": "Shortcuts",
    "description": "Settings > Agents: row that opens the page where you manage the quick action chips of a new chat."
  },
  "settings.agents.shortcutsHelp": {
    "message": "What a new chat offers, with every agent",
    "description": "Settings > Agents: description under the Shortcuts row."
  },
  "settings.shortcuts.title": {
    "message": "Shortcuts",
    "description": "Settings > Agents > Shortcuts: page title."
  },
  "settings.shortcuts.help": {
    "message": "A new chat shows these shortcuts under the agent's card, with every agent. Each one is a / command in the chat input too. A shortcut you turn off leaves new chats, but its command keeps working.",
    "description": "Settings > Agents > Shortcuts: intro text above the list."
  },
  "settings.shortcuts.add": {
    "message": "Add a shortcut",
    "description": "Settings > Agents > Shortcuts: row under the list that opens the editor for a new shortcut of your own."
  },
  "settings.shortcuts.added": {
    "message": "Added {name}",
    "description": "Settings > Agents > Shortcuts: toast after you added a shortcut. name: its label."
  },
  "settings.shortcuts.reset": {
    "message": "Restore Rukoo's shortcuts",
    "description": "Settings > Agents > Shortcuts: row that puts back Rukoo's own shortcuts, as they came."
  },
  "settings.shortcuts.resetHelp": {
    "message": "Removes your own shortcuts and your changes to Rukoo's",
    "description": "Settings > Agents > Shortcuts: description under the restore row."
  },
  "settings.shortcuts.resetTitle": {
    "message": "Restore Rukoo's shortcuts?",
    "description": "Settings > Agents > Shortcuts: title of the dialog that confirms the restore."
  },
  "settings.shortcuts.resetConfirm": {
    "message": "Your own shortcuts go, and Rukoo's get their own names, instructions and order back.",
    "description": "Settings > Agents > Shortcuts: text of the dialog that confirms the restore."
  },
  "settings.shortcuts.resetOk": {
    "message": "Restore",
    "description": "Settings > Agents > Shortcuts: confirm button of the restore dialog."
  },
  "settings.shortcuts.resetDone": {
    "message": "Rukoo's shortcuts are back",
    "description": "Settings > Agents > Shortcuts: toast after the restore."
  },
  "settings.shortcuts.moveUp": {
    "message": "Move {name} up",
    "description": "Settings > Agents > Shortcuts: tooltip and accessible name of the up arrow in a shortcut's row. name: the shortcut's label."
  },
  "settings.shortcuts.moveDown": {
    "message": "Move {name} down",
    "description": "Settings > Agents > Shortcuts: tooltip and accessible name of the down arrow in a shortcut's row. name: the shortcut's label."
  },
  "settings.shortcuts.show": {
    "message": "Show {name} in new chats",
    "description": "Settings > Agents > Shortcuts: tooltip and accessible name of the switch in a shortcut's row. name: the shortcut's label."
  },
  "settings.shortcuts.whenEmail": {
    "message": "With an open email",
    "description": "Settings > Agents > Shortcuts: in a shortcut's row, after its /command: it shows in a new chat about an email."
  },
  "settings.shortcuts.whenNoEmail": {
    "message": "Without an open email",
    "description": "Settings > Agents > Shortcuts: in a shortcut's row, after its /command: it shows in a new chat when no email is open (the inbox overview)."
  },
  "settings.shortcuts.whenNewsletter": {
    "message": "With a newsletter open",
    "description": "Settings > Agents > Shortcuts: in a shortcut's row, after its /command: it shows for email you can unsubscribe from."
  },
  "settings.shortcuts.whenAlways": {
    "message": "Always",
    "description": "Settings > Agents > Shortcuts: in a shortcut's row, after its /command: it shows in every new chat, with or without an email."
  },
  "settings.shortcuts.edited": {
    "message": "Edited",
    "description": "Settings > Agents > Shortcuts: in a row of one of Rukoo's shortcuts whose label or instructions you changed."
  },
  "settings.shortcuts.editTitle": {
    "message": "Edit shortcut",
    "description": "Settings > Agents > Shortcuts: title of the editor dialog, and tooltip of a shortcut's row."
  },
  "settings.shortcuts.newTitle": {
    "message": "New shortcut",
    "description": "Settings > Agents > Shortcuts: title of the editor dialog for a shortcut of your own."
  },
  "settings.shortcuts.label": {
    "message": "Name",
    "description": "Settings > Agents > Shortcuts editor: label of the field for the text on the chip, such as Draft a reply."
  },
  "settings.shortcuts.command": {
    "message": "Command",
    "description": "Settings > Agents > Shortcuts editor: label of the field for the / command, such as /reply."
  },
  "settings.shortcuts.commandPlaceholder": {
    "message": "follow-up",
    "description": "Settings > Agents > Shortcuts editor: example in the empty command field, after the /. Lowercase letters, digits and hyphens only."
  },
  "settings.shortcuts.commandHelp": {
    "message": "Lowercase letters, digits and hyphens. Type it after a / in the chat input.",
    "description": "Settings > Agents > Shortcuts editor: help under the command field of your own shortcut."
  },
  "settings.shortcuts.commandFixed": {
    "message": "Rukoo's shortcuts keep their command.",
    "description": "Settings > Agents > Shortcuts editor: help under the disabled command field of one of Rukoo's shortcuts."
  },
  "settings.shortcuts.when": {
    "message": "Show it",
    "description": "Settings > Agents > Shortcuts editor: label above the two choices of when your shortcut shows in a new chat."
  },
  "settings.shortcuts.whenEmailOption": {
    "message": "When an email is open",
    "description": "Settings > Agents > Shortcuts editor: choice under Show it."
  },
  "settings.shortcuts.whenAlwaysOption": {
    "message": "Always, also without an email",
    "description": "Settings > Agents > Shortcuts editor: choice under Show it."
  },
  "settings.shortcuts.icon": {
    "message": "Icon",
    "description": "Settings > Agents > Shortcuts editor: label above the icons you can pick for your shortcut's chip."
  },
  "settings.shortcuts.prompt": {
    "message": "Instructions for the agent",
    "description": "Settings > Agents > Shortcuts editor: label of the text the agent gets when you pick the shortcut."
  },
  "settings.shortcuts.promptHelp": {
    "message": "The agent gets this when you pick the shortcut, with the open email if there is one.",
    "description": "Settings > Agents > Shortcuts editor: help under the instructions of your own shortcut."
  },
  "settings.shortcuts.promptHelpBuiltin": {
    "message": "The agent gets this when you pick the shortcut. Rukoo writes its own instructions in English, for the model.",
    "description": "Settings > Agents > Shortcuts editor: help under the instructions of one of Rukoo's shortcuts."
  },
  "settings.shortcuts.restore": {
    "message": "Restore default",
    "description": "Settings > Agents > Shortcuts editor: button that gives one of Rukoo's shortcuts its own name and instructions back."
  },
  "settings.shortcuts.needLabel": {
    "message": "Give the shortcut a name.",
    "description": "Settings > Agents > Shortcuts editor: error when the name field is empty."
  },
  "settings.shortcuts.needPrompt": {
    "message": "Write what the agent should do.",
    "description": "Settings > Agents > Shortcuts editor: error when the instructions are empty."
  },
  "settings.shortcuts.badCommand": {
    "message": "Use lowercase letters, digits and single hyphens, up to 40.",
    "description": "Settings > Agents > Shortcuts editor: error for a command with other characters."
  },
  "settings.shortcuts.takenCommand": {
    "message": "/{name} is already a command.",
    "description": "Settings > Agents > Shortcuts editor: error when another shortcut, or Rukoo itself, has this command. name: the command without its slash."
  },
  "settings.shortcuts.tooMany": {
    "message": "Rukoo keeps up to {count} shortcuts.",
    "description": "Settings > Agents > Shortcuts: toast when you try to add one more than the limit. count: the limit."
  },
  "settings.aliases.add": {
    "message": "Add alias",
    "description": "settings > aliases. src/renderer/settings.js (render); src/renderer/settings.js (openSettings)"
  },
  "settings.aliases.count": {
    "message": {
      "one": "{count} alias",
      "other": "{count} aliases"
    },
    "description": "settings > aliases. src/renderer/settings.js (render)"
  },
  "settings.aliases.fetch": {
    "message": "Fetch from Gmail",
    "description": "settings > aliases. src/renderer/settings.js (render)"
  },
  "settings.aliases.fetched": {
    "message": {
      "one": "{count} alias fetched from Gmail",
      "other": "{count} aliases fetched from Gmail"
    },
    "description": "settings > aliases. src/renderer/settings.js (openSettings)"
  },
  "settings.aliases.fetchHelp": {
    "message": "Import verified addresses from Gmail's \"Send mail as\" settings.",
    "description": "settings > aliases. src/renderer/settings.js (render)"
  },
  "settings.aliases.help": {
    "message": "Your server must allow sending from this address. In Gmail, see Settings > Accounts > Send mail as.",
    "description": "settings > aliases. src/renderer/settings.js (render)"
  },
  "settings.aliases.makeDefault": {
    "message": "Set as default",
    "description": "settings > aliases. src/renderer/settings.js (openSettings)"
  },
  "settings.aliases.manageHint": {
    "message": "Click to set as default or remove",
    "description": "settings > aliases. src/renderer/settings.js (render)"
  },
  "settings.aliases.primary": {
    "message": "Account address",
    "description": "settings > aliases. src/renderer/settings.js (render)"
  },
  "settings.aliases.sendAs": {
    "message": "Send as",
    "description": "settings > aliases. src/renderer/settings.js (render)"
  },
  "settings.badge.new": {
    "message": "New emails",
    "description": "settings > badge. src/renderer/settings.js"
  },
  "settings.badge.unread": {
    "message": "Unread emails",
    "description": "settings > badge. src/renderer/settings.js"
  },
  "settings.colors.blue": {
    "message": "Blue",
    "description": "settings > colors. src/renderer/settings.js"
  },
  "settings.colors.lightBlue": {
    "message": "Light blue",
    "description": "settings > colors. src/renderer/settings.js"
  },
  "settings.colors.orange": {
    "message": "Orange",
    "description": "settings > colors. src/renderer/settings.js"
  },
  "settings.colors.purple": {
    "message": "Purple",
    "description": "settings > colors. src/renderer/settings.js"
  },
  "settings.colors.red": {
    "message": "Red",
    "description": "settings > colors. src/renderer/settings.js"
  },
  "settings.colors.turquoise": {
    "message": "Turquoise",
    "description": "settings > colors. src/renderer/settings.js"
  },
  "settings.colors.yellow": {
    "message": "Yellow",
    "description": "settings > colors. src/renderer/settings.js"
  },
  "settings.density.compact": {
    "message": "Compact",
    "description": "settings > density. src/renderer/settings.js"
  },
  "settings.density.compactHint": {
    "message": "One line per email",
    "description": "settings > density. src/renderer/settings.js"
  },
  "settings.density.standard": {
    "message": "Standard",
    "description": "settings > density. src/renderer/settings.js"
  },
  "settings.density.standardHint": {
    "message": "Sender, subject, and preview",
    "description": "settings > density. src/renderer/settings.js"
  },
  "settings.folders.show": {
    "message": "Show in sidebar",
    "description": "settings > folders. src/renderer/settings.js (render)"
  },
  "settings.general.badge": {
    "message": "App badge count",
    "description": "settings > general. src/renderer/settings.js (render); src/renderer/settings.js (openSettings)"
  },
  "settings.general.closeToTray": {
    "message": "Close to the tray",
    "description": "settings > general. Settings > General: title of the switch that keeps Rukoo running in the Windows notification area when the main window closes. On by default. src/renderer/settings.js (render)"
  },
  "settings.general.closeToTrayHelp": {
    "message": "Rukoo keeps running in the tray when you close the window, so new mail still comes in and notifies you. To quit, right-click the tray icon and choose Quit.",
    "description": "settings > general. Settings > General: explanation below Close to the tray; Quit is the label of native.tray.quit. src/renderer/settings.js (render)"
  },
  "settings.general.darkEmails": {
    "message": "Dark email display",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.darkEmailsHelp": {
    "message": "Adjust HTML email colors in dark mode.",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.density": {
    "message": "Message list",
    "description": "settings > general. src/renderer/settings.js (render); src/renderer/settings.js (openSettings)"
  },
  "settings.general.stackSenders": {
    "message": "Group mail from one sender",
    "description": "Settings > General: switch that folds a run of emails from one sender into one row of the message list, with a +N count. Off by default. src/renderer/settings.js (render)"
  },
  "settings.general.stackSendersHelp": {
    "message": "Three or more read emails in a row from the same sender, on the same day, share a row in the list. Unread mail always gets its own row.",
    "description": "Settings > General: explanation under the switch that groups mail from one sender. src/renderer/settings.js (render)"
  },
  "settings.general.fit": {
    "message": "Fit content to window",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.fitHelp": {
    "message": "Shrink wide email to fit the window, down to 80%. Anything still too wide scrolls sideways.",
    "description": "settings > general, help under the \"Fit content to window\" toggle. 80% is MIN_ZOOM in src/renderer/mailframe.js (fillFrame). src/renderer/settings.js (render)"
  },
  "settings.general.folders": {
    "message": "Manage folders",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.foldersHelp": {
    "message": "Show or hide email folders.",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.language": {
    "message": "Language",
    "description": "settings > general. Settings > General: opens the interface language picker."
  },
  "settings.general.languageHelp": {
    "message": "Choose the language used throughout the app.",
    "description": "settings > general. Settings > General: explanation below Language."
  },
  "settings.general.logos": {
    "message": "Sender logos",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.logosHelp": {
    "message": "Show logos for companies that email you. Rukoo Mail fetches each logo once from their website.",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.mailtoLinuxHelp": {
    "message": "Open mailto: links from your browser and other apps in Rukoo Mail. Uses your desktop’s default email app setting.",
    "description": "Linux email-link integration in Settings > General. src/renderer/settings.js and src/main/main.js."
  },
  "settings.general.mailtoLinuxUnavailable": {
    "message": "Install the .deb package to use Rukoo Mail for email links.",
    "description": "Linux email-link integration in Settings > General. src/renderer/settings.js and src/main/main.js."
  },
  "settings.general.mailtoLinuxFailed": {
    "message": "Could not set the default email app. Install xdg-utils or choose Rukoo Mail in your desktop’s default applications settings.",
    "description": "Linux email-link integration in Settings > General. src/renderer/settings.js and src/main/main.js."
  },
  "settings.general.mailto": {
    "message": "Email links",
    "description": "settings > general, title of the row that makes Rukoo Mail the app for mailto: links. src/renderer/settings.js (mailtoRow)"
  },
  "settings.general.mailtoAppDescription": {
    "message": "Email, calendar and agent chat for Windows",
    "description": "Description Windows may show for Rukoo Mail among the apps that open mailto: links. src/main/main.js (mailClient)"
  },
  "settings.general.mailtoHandler": {
    "message": "Opened by {app}",
    "description": "settings > general, under Email links: the app that opens mailto: links now. {app} is its name from Windows, such as Firefox or Outlook. src/renderer/settings.js (mailtoRow)"
  },
  "settings.general.mailtoHelp": {
    "message": "Open mailto: links from your browser and other apps in Rukoo Mail. Windows asks you to pick it under Default apps.",
    "description": "settings > general, help under Email links. \"Default apps\" is the Windows Settings page. src/renderer/settings.js (mailtoRow)"
  },
  "settings.general.mailtoNone": {
    "message": "No app opens them yet",
    "description": "settings > general, under Email links: Windows has no app for mailto: links. src/renderer/settings.js (mailtoRow)"
  },
  "settings.general.mailtoRukoo": {
    "message": "Rukoo Mail opens them",
    "description": "settings > general, under Email links: Rukoo Mail is the app for mailto: links. src/renderer/settings.js (mailtoRow)"
  },
  "settings.general.mailtoUnavailable": {
    "message": "Only in the installed or portable app",
    "description": "settings > general, under Email links, in a run from source or a test profile, which Windows can't offer for links. Also the error if the button is used there. src/renderer/settings.js (mailtoRow), src/main/main.js (useForMailto)"
  },
  "settings.general.mailtoUse": {
    "message": "Use Rukoo Mail",
    "description": "settings > general, button under Email links: offers Rukoo Mail to Windows for mailto: links and opens the Windows page to pick it. src/renderer/settings.js (mailtoRow)"
  },
  "settings.general.minimizeToTray": {
    "message": "Minimize to the tray",
    "description": "settings > general. Settings > General: title of the switch that hides the minimized main window in the Windows notification area instead of the taskbar. Off by default. src/renderer/settings.js (render)"
  },
  "settings.general.minimizeToTrayHelp": {
    "message": "Minimizing puts the window in the tray instead of on the taskbar.",
    "description": "settings > general. Settings > General: explanation below Minimize to the tray. src/renderer/settings.js (render)"
  },
  "settings.general.notifications": {
    "message": "Notifications",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.notificationsHelp": {
    "message": "Show a desktop notification for new emails.",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.signature": {
    "message": "Signature",
    "description": "settings > general. src/renderer/settings.js (render); src/renderer/settings.js (openSettings)"
  },
  "settings.general.spam": {
    "message": "Spam addresses",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.spamHelp": {
    "message": "Edit your list of spam senders.",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.swipe": {
    "message": "Touchscreen swipe actions",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.swipeHelp": {
    "message": "Swipe right to mark as read or unread, and left to delete.",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.sync": {
    "message": "Sync schedule",
    "description": "settings > general. src/renderer/settings.js (render); src/renderer/settings.js (openSettings)"
  },
  "settings.general.theme": {
    "message": "Theme",
    "description": "settings > general. src/renderer/settings.js (render); src/renderer/settings.js (openSettings)"
  },
  "settings.general.vipHelp": {
    "message": "Emails from VIPs appear in the VIPs folder.",
    "description": "settings > general. src/renderer/settings.js (render)"
  },
  "settings.general.zoom": {
    "message": "Zoom",
    "description": "settings > general. Settings > General: title of the zoom row, which shows the zoom level of the main window and the compose windows. src/renderer/settings.js (zoomRow)"
  },
  "settings.general.zoomHelp": {
    "message": "Use Ctrl+Plus and Ctrl+Minus, or Ctrl with the mouse wheel. Ctrl+0 goes back to 100%.",
    "description": "settings > general. Settings > General: explanation below Zoom, naming the keyboard shortcuts and the mouse wheel. src/renderer/settings.js (zoomRow)"
  },
  "settings.general.zoomReset": {
    "message": "Reset to 100%",
    "description": "settings > general. Settings > General: button in the zoom row that sets the zoom back to 100%; disabled at 100%. src/renderer/settings.js (zoomRow)"
  },
  "settings.general.zoomValue": {
    "message": "{percent}%",
    "description": "settings > general. Settings > General: current zoom level in the zoom row; percent is a whole number such as 125. src/renderer/settings.js (zoomRow, showZoom)"
  },
  "settings.groups.about": {
    "message": "About",
    "description": "settings > groups. src/renderer/settings.js (render)"
  },
  "settings.groups.account": {
    "message": "Account",
    "description": "settings > groups. src/renderer/settings.js (render)"
  },
  "settings.groups.accounts": {
    "message": "Accounts",
    "description": "settings > groups. src/renderer/settings.js (render)"
  },
  "settings.groups.agents": {
    "message": "Agents",
    "description": "Settings overview: heading of the group with the chat agents."
  },
  "settings.groups.general": {
    "message": "General",
    "description": "settings > groups. src/renderer/settings.js (render)"
  },
  "settings.server.checking": {
    "message": "Checking...",
    "description": "settings > server. Server settings form: progress label while checking credentials."
  },
  "settings.server.passwordHint": {
    "message": "Leave blank to keep the current password",
    "description": "settings > server. src/renderer/settings.js (render)"
  },
  "settings.server.saved": {
    "message": "Server settings saved",
    "description": "settings > server. src/renderer/settings.js (openSettings)"
  },
  "settings.server.caldav": {
    "message": "Calendar server (CalDAV)",
    "description": "Server settings form and the manual servers of the setup form: label of the optional field for the address of the account's CalDAV calendar server. src/renderer/settings.js (render); src/renderer/setup.js (showLogin)"
  },
  "settings.server.caldavPlaceholder": {
    "message": "Optional, for example caldav.example.com",
    "description": "Server settings form and the manual servers of the setup form: placeholder in the empty CalDAV address field. src/renderer/settings.js (render); src/renderer/setup.js (showLogin)"
  },
  "settings.server.caldavHelp": {
    "message": "Rukoo signs in with the username and password above. Leave this empty to show no calendar for this account.",
    "description": "Server settings form and the manual servers of the setup form: help text under the CalDAV address field. In both forms the username and password fields are above it. src/renderer/settings.js (render); src/renderer/setup.js (showLogin)"
  },
  "settings.signature.generalValue": {
    "message": "General: {signature}",
    "description": "settings > signature. src/renderer/settings.js (render)"
  },
  "settings.signature.help": {
    "message": "Paste a signature from another mail app or a web page, or write and format it here.",
    "description": "settings > signature. Help text above the signature editor in the Signature dialog. src/renderer/signature.js (editSignature)"
  },
  "settings.signature.placeholder": {
    "message": "Your name, role and contact details",
    "description": "settings > signature. Placeholder in the empty signature editor. src/renderer/signature.js (editSignature)"
  },
  "settings.signature.source": {
    "message": "Edit HTML",
    "description": "settings > signature. Tooltip and accessible name of the toolbar button that switches the signature editor to its HTML source, and of the source field. src/renderer/signature.js (editSignature)"
  },
  "settings.signature.tooLarge": {
    "message": "This signature is too large. Use smaller images, or images from the web.",
    "description": "settings > signature. Error when a pasted or saved signature exceeds the size limit, mostly because of embedded images. src/main/signature.js (cleanSignatureHtml)"
  },
  "settings.signature.usesGeneral": {
    "message": "Uses the general signature",
    "description": "settings > signature. Account settings: the account has no signature of its own and uses the general one, which is previewed below. src/renderer/settings.js (render)"
  },
  "settings.signature.useGeneral": {
    "message": "Use general signature",
    "description": "settings > signature. src/renderer/settings.js (openSettings)"
  },
  "calendar.free.title": {
    "message": "Availability",
    "description": "Title of the calendar's availability side panel, which lists open time across the calendars that count toward availability; also the name of its toolbar button. src/renderer/calendar-free.js"
  },
  "calendar.free.length": {
    "message": "Length",
    "description": "Availability panel and the composer's free-times dialog: label of the meeting length choice (15 to 90 minutes, or the meeting type's length). src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.range": {
    "message": "Period",
    "description": "Availability panel and the composer's free-times dialog: label of the choice of period to look in. src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.rangeThisWeek": {
    "message": "This week",
    "description": "Period option: from now to the end of this week. src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.rangeNextWeek": {
    "message": "Next week",
    "description": "Period option: the whole of next week. src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.range14": {
    "message": "Next 14 days",
    "description": "Period option: from now to 14 days ahead. src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.hours": {
    "message": "Hours",
    "description": "Label of the choice of which hours count as open: booking page hours, working hours or any time. src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.hoursBooking": {
    "message": "Booking hours",
    "description": "Hours option: the hours set for the booking page; only offered while the booking page is on. src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.hoursWorking": {
    "message": "Working hours",
    "description": "Hours option: the working hours from Settings > Calendar. src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.hoursAny": {
    "message": "Any time",
    "description": "Hours option: the whole day and night count as open. src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.zone": {
    "message": "Time zone",
    "description": "Label of the time zone the open times are shown in. src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.zoneHere": {
    "message": "{zone} (this computer)",
    "description": "Time zone list: the zone this computer is set to, listed first; zone is an IANA name such as Europe/Amsterdam. src/renderer/calendar-shared.js (zoneOptions)"
  },
  "calendar.free.checked": {
    "message": { "one": "Checked: {count} calendar", "other": "Checked: {count} calendars" },
    "description": "Availability panel: link that shows which calendars were read for busy time; count is how many. src/renderer/calendar-free.js"
  },
  "calendar.free.checkedNone": {
    "message": "No calendar counts toward availability, so every time looks open.",
    "description": "Availability panel: warning when no calendar counts toward availability. src/renderer/calendar-free.js"
  },
  "calendar.free.checkedChange": {
    "message": "Change in Settings",
    "description": "Availability panel: link to Settings > Calendar, where calendars are set to count toward availability. src/renderer/calendar-free.js"
  },
  "calendar.free.failed": {
    "message": "Couldn't check {email}. These times may be wrong.",
    "description": "Warning in the availability panel and the free-times preview for an account whose calendars couldn't be read; email is that account's address. src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.empty": {
    "message": "No open time of {length} in this period.",
    "description": "Availability panel: nothing found; length is the chosen length, such as \"30 min\". src/renderer/calendar-free.js"
  },
  "calendar.free.loading": {
    "message": "Looking for open time…",
    "description": "Shown while the open times are worked out, in the availability panel and the free-times dialog. src/renderer/calendar-free.js, src/renderer/composer.js"
  },
  "calendar.free.allDay": {
    "message": "All day: {title}",
    "description": "Availability panel: an all-day event on that day, shown under the day even when it doesn't make you busy; title is the event's title. src/renderer/calendar-free.js"
  },
  "calendar.free.copyText": {
    "message": "Copy as text",
    "description": "Availability panel button: copies the open times as lines of text. src/renderer/calendar-free.js"
  },
  "calendar.free.copied": {
    "message": "Open times copied",
    "description": "Toast after Copy as text. src/renderer/calendar-free.js"
  },
  "calendar.free.copyLink": {
    "message": "Copy booking link",
    "description": "Availability panel button: copies the booking page's address; only while the page is on. src/renderer/calendar-free.js"
  },
  "calendar.free.linkCopied": {
    "message": "Booking link copied",
    "description": "Toast after Copy booking link. src/renderer/calendar-free.js"
  },
  "calendar.free.block": {
    "message": "Block this day",
    "description": "Availability panel: link beside a day that stops the booking page from offering times on it; only while the page is on. src/renderer/calendar-free.js"
  },
  "calendar.free.blockLabel": {
    "message": "Block {date} on your booking page",
    "description": "Accessible name of Block this day; date is the day, such as \"Tuesday 3 November\". src/renderer/calendar-free.js"
  },
  "calendar.free.blocked": {
    "message": "{date} is blocked on your booking page",
    "description": "Toast after Block this day, with Undo; date is the day. src/renderer/calendar-free.js"
  },
  "calendar.free.blockZone": {
    "message": "Days on your booking page are blocked in its time zone.",
    "description": "Availability panel: note under the days when they are shown in another time zone than the booking page's, so Block this day isn't offered; followed by calendar.free.blockZoneAction. src/renderer/calendar-free.js"
  },
  "calendar.free.blockZoneAction": {
    "message": "Show times in {zone}",
    "description": "Availability panel: link after calendar.free.blockZone that shows the days in the booking page's time zone; zone is an IANA name such as America/Los Angeles. src/renderer/calendar-free.js"
  },
  "calendar.free.show": {
    "message": "Show this day in the calendar",
    "description": "Tooltip of an open time in the availability panel; a click moves the calendar to that day. src/renderer/calendar-free.js"
  },
  "calendar.free.zoneLine": {
    "message": "Times in {zone}",
    "description": "Last line of the text Copy as text puts on the clipboard; zone is an IANA name such as Europe/Amsterdam. src/renderer/calendar-free.js"
  },
  "composer.booking.button": {
    "message": "Insert from calendar",
    "description": "Tooltip of the calendar button in the composer's bottom bar, which opens a menu to insert the booking link or free times. src/renderer/composer.js"
  },
  "composer.booking.insertLink": {
    "message": "Insert booking link",
    "description": "Composer calendar menu: inserts the booking page's address; only while the page is on. src/renderer/composer.js"
  },
  "composer.booking.insertTimes": {
    "message": "Insert my free times…",
    "description": "Composer calendar menu: opens the dialog that inserts a list of free times. src/renderer/composer.js"
  },
  "composer.booking.timesTitle": {
    "message": "Insert free times",
    "description": "Title of the composer's free-times dialog. src/renderer/composer.js"
  },
  "composer.booking.type": {
    "message": "Meeting type",
    "description": "Free-times dialog: choice of the booking page's meeting type, whose length the times then use; shown instead of Length when the booking link goes in too. src/renderer/composer.js"
  },
  "composer.booking.withLink": {
    "message": "Add my booking link",
    "description": "Free-times dialog checkbox: also inserts the booking page's link under the times. src/renderer/composer.js"
  },
  "composer.booking.linkInFooter": {
    "message": "The link is under your signature already, so it isn't added twice.",
    "description": "Free-times dialog: added to the help under Add my booking link when the message's signature footer has the booking link. src/renderer/composer.js (freeTimes)"
  },
  "composer.booking.withLinkHelp": {
    "message": "The times then follow your booking page's hours, and the meeting type's length and minimum notice. The page may offer fewer, because free time around meetings and the limit per day only count there.",
    "description": "Free-times dialog: help under Add my booking link. src/renderer/composer.js"
  },
  "composer.booking.preview": {
    "message": "Preview",
    "description": "Free-times dialog: heading of the preview of what will be inserted. src/renderer/composer.js"
  },
  "composer.booking.insert": {
    "message": "Insert",
    "description": "Free-times dialog: the button that puts the times into the message. src/renderer/composer.js"
  },
  "composer.booking.none": {
    "message": "No open time in this period. Try another period, length or hours.",
    "description": "Free-times dialog preview when nothing is open. src/renderer/composer.js"
  },
  "composer.booking.intro": {
    "message": "I'm free at these times:",
    "description": "First line of the free times inserted into a message, read by its recipient. src/renderer/composer.js"
  },
  "composer.booking.zoneLine": {
    "message": "Times are in {zone}.",
    "description": "Line under the free times inserted into a message; zone is an IANA name such as Europe/Amsterdam. src/renderer/composer.js"
  },
  "composer.booking.linkLine": {
    "message": "Or pick a time that suits you: {url}",
    "description": "Line under the free times inserted into a message when the booking link goes in too; url becomes the link, with the address as its text. src/renderer/composer.js"
  },
  "composer.booking.footerDefault": {
    "message": "Want to meet with me? Pick a time, or share this link with your agent: {url}",
    "description": "Default text of the booking link under the signature, in the booking page's language, read by the mail's recipient; url becomes the link, with the address as its text. \"Agent\" means an AI assistant. src/renderer/composer.js (bookingFooterHtml)"
  },
  "settings.groups.calendar": {
    "message": "Calendar",
    "description": "Settings overview: heading of the group with the Calendar and Booking page rows. src/renderer/settings.js (render)"
  },
  "settings.calendar.title": {
    "message": "Calendar",
    "description": "Title of Settings > Calendar and of its row on the Settings overview. src/renderer/calendar-settings.js"
  },
  "settings.calendar.rowDesc": {
    "message": "Week, working hours, reminders and which calendars count",
    "description": "Settings overview: second line of the Calendar row. src/renderer/calendar-settings.js"
  },
  "settings.video.title": {
    "message": "Video meetings",
    "description": "Title of Settings > Video meetings, where Zoom, Microsoft Teams and Jitsi Meet are connected for bookings, and of its row on the Settings overview. src/renderer/video-settings.js"
  },
  "settings.video.rowNone": {
    "message": "Give each booking its own Zoom, Teams or Jitsi link",
    "description": "Settings overview: second line of the Video meetings row while no service is connected. src/renderer/video-settings.js"
  },
  "settings.video.rowConnected": {
    "message": "Connected: {services}",
    "description": "Settings overview: second line of the Video meetings row. services lists the connected ones, such as 'Zoom and Jitsi Meet'. src/renderer/video-settings.js"
  },
  "settings.video.intro": {
    "message": "Connect a service, then choose it as a meeting type's location on your booking page. Each booking then gets its own meeting link in the guest's invitation.",
    "description": "Settings > Video meetings: text at the top. src/renderer/video-settings.js"
  },
  "settings.video.thisComputer": {
    "message": "Sign-ins stay on this computer. When another computer handles your bookings, connect the services there too.",
    "description": "Settings > Video meetings: note under the services. src/renderer/video-settings.js"
  },
  "settings.video.services": {
    "message": "Services",
    "description": "Settings > Video meetings: group heading above the list of services. src/renderer/video-settings.js"
  },
  "settings.video.jitsiHelp": {
    "message": "No account needed. Each booking gets its own room, with a name nobody can guess.",
    "description": "Settings > Video meetings: line under Jitsi Meet, which has a switch. src/renderer/video-settings.js"
  },
  "settings.video.jitsiServer": {
    "message": "Jitsi server",
    "description": "Settings > Video meetings: the row and dialog title for the Jitsi server the rooms are on. src/renderer/video-settings.js"
  },
  "settings.video.jitsiServerHelp": {
    "message": "Leave empty to use meet.jit.si.",
    "description": "Settings > Video meetings: help in the Jitsi server dialog. src/renderer/video-settings.js"
  },
  "settings.video.zoomHelp": {
    "message": "Makes a meeting in your Zoom account for each booking.",
    "description": "Settings > Video meetings: line under Zoom while it isn't connected. src/renderer/video-settings.js"
  },
  "settings.video.teamsHelp": {
    "message": "Makes a Teams meeting for each booking. Needs a work or school account with Microsoft Teams.",
    "description": "Settings > Video meetings: line under Microsoft Teams while it isn't connected. src/renderer/video-settings.js"
  },
  "settings.video.connectedAs": {
    "message": "Connected as {account}",
    "description": "Settings > Video meetings: line under a connected Zoom or Teams. account is the address or name of the signed-in account. src/renderer/video-settings.js"
  },
  "settings.video.connectedPlain": {
    "message": "Connected",
    "description": "Settings > Video meetings: line under a connected Zoom or Teams when the service didn't say which account. src/renderer/video-settings.js"
  },
  "settings.video.renewHelp": {
    "message": "The connection stopped working. Renew it to keep adding links to bookings.",
    "description": "Settings > Video meetings: line under Zoom or Teams when its stored sign-in was refused or expired. src/renderer/video-settings.js"
  },
  "settings.video.unavailable": {
    "message": "This copy of Rukoo has no {service} app to sign in with. Add a client file, as the booking page guide explains.",
    "description": "Settings > Video meetings: line under Zoom or Teams when no OAuth client is configured. service is Zoom or Microsoft Teams. src/renderer/video-settings.js"
  },
  "settings.video.pending": {
    "message": "Finish signing in to {service} in your browser.",
    "description": "Settings > Video meetings: line under Zoom or Teams while its browser sign-in is open. src/renderer/video-settings.js"
  },
  "settings.video.connect": {
    "message": "Connect",
    "description": "Settings > Video meetings: button that signs in to Zoom or Microsoft Teams in the browser. src/renderer/video-settings.js"
  },
  "settings.video.renew": {
    "message": "Renew",
    "description": "Settings > Video meetings: button that signs in to Zoom or Teams again after the connection stopped working. src/renderer/video-settings.js"
  },
  "settings.video.disconnect": {
    "message": "Disconnect",
    "description": "Settings > Video meetings: button that forgets the sign-in to Zoom or Teams on this computer. src/renderer/video-settings.js"
  },
  "settings.video.addClient": {
    "message": "Add client file…",
    "description": "Settings > Video meetings: button that opens a file picker for a Zoom or Microsoft OAuth client file (JSON). src/renderer/video-settings.js"
  },
  "settings.video.importTitle": {
    "message": "Choose the {service} client file",
    "description": "Title of the file picker for a video service's OAuth client file. service is Zoom or Microsoft Teams. src/main/main.js (videoImportClient)"
  },
  "settings.video.connected": {
    "message": "{service} connected",
    "description": "Toast after signing in to a video service. service is Zoom or Microsoft Teams. src/renderer/video-settings.js"
  },
  "settings.video.disconnected": {
    "message": "{service} disconnected",
    "description": "Toast after disconnecting a video service. service is Zoom or Microsoft Teams. src/renderer/video-settings.js"
  },
  "settings.video.clientAdded": {
    "message": "Client file added",
    "description": "Toast after a video service's OAuth client file was chosen and stored. src/renderer/video-settings.js"
  },
  "settings.video.disconnectTitle": {
    "message": "Disconnect {service}?",
    "description": "Title of the confirmation before disconnecting a video service. service is Zoom or Microsoft Teams. src/renderer/video-settings.js"
  },
  "settings.video.disconnectConfirm": {
    "message": "Rukoo forgets the sign-in on this computer. New bookings that use {service} go in your calendar without a link until you connect it again. Meetings already made stay.",
    "description": "Text of the confirmation before disconnecting a video service. service is Zoom or Microsoft Teams. src/renderer/video-settings.js"
  },
  "settings.video.serverSaved": {
    "message": "Jitsi server saved",
    "description": "Toast after changing the Jitsi server in Settings > Video meetings. src/renderer/video-settings.js"
  },
  "settings.calendar.week": {
    "message": "Week and working hours",
    "description": "Settings > Calendar: group heading. src/renderer/calendar-settings.js"
  },
  "settings.calendar.weekStart": {
    "message": "Week starts on",
    "description": "Settings > Calendar: row and dialog title for the first day of the week. src/renderer/calendar-settings.js"
  },
  "settings.calendar.weekStartAuto": {
    "message": "Automatic ({day})",
    "description": "Week starts on: the language's own first day; day is that weekday's name. src/renderer/calendar-settings.js"
  },
  "settings.calendar.weekends": {
    "message": "Show weekends",
    "description": "Settings > Calendar switch. src/renderer/calendar-settings.js"
  },
  "settings.calendar.weekendsHelp": {
    "message": "Off: a week shows only Monday to Friday.",
    "description": "Settings > Calendar: help under Show weekends. src/renderer/calendar-settings.js"
  },
  "settings.calendar.workDays": {
    "message": "Working days",
    "description": "Settings > Calendar: row with a toggle button per weekday. src/renderer/calendar-settings.js"
  },
  "settings.calendar.workHours": {
    "message": "Working hours",
    "description": "Settings > Calendar: row with the start and end time of the working day. src/renderer/calendar-settings.js"
  },
  "settings.calendar.workHoursHelp": {
    "message": "The calendar dims the time outside them. Free time and your booking page use them too.",
    "description": "Settings > Calendar: help under Working hours. src/renderer/calendar-settings.js"
  },
  "settings.calendar.workFrom": {
    "message": "From",
    "description": "Accessible name of a start time field (working hours, a booking date's hours). src/renderer/calendar-settings.js, src/renderer/booking-settings.js"
  },
  "settings.calendar.workTo": {
    "message": "To",
    "description": "Accessible name of an end time field (working hours, a booking date's hours). src/renderer/calendar-settings.js, src/renderer/booking-settings.js"
  },
  "settings.calendar.workInvalid": {
    "message": "The end of the day has to come after its start.",
    "description": "Settings > Calendar: error under working hours that end before they start. src/renderer/calendar-settings.js"
  },
  "settings.calendar.noWorkDays": {
    "message": "No working days",
    "description": "Working hours as text when no weekday is a working day. src/renderer/calendar-settings.js"
  },
  "settings.calendar.events": {
    "message": "New events and reminders",
    "description": "Settings > Calendar: group heading. src/renderer/calendar-settings.js"
  },
  "settings.calendar.duration": {
    "message": "Default event length",
    "description": "Settings > Calendar: row and dialog title; the length a new event gets with one click. src/renderer/calendar-settings.js"
  },
  "settings.calendar.notify": {
    "message": "Event reminders",
    "description": "Settings > Calendar switch: desktop notifications before events. src/renderer/calendar-settings.js"
  },
  "settings.calendar.notifyHelp": {
    "message": "A notification on your desktop before an event starts.",
    "description": "Settings > Calendar: help under Event reminders. src/renderer/calendar-settings.js"
  },
  "settings.calendar.reminder": {
    "message": "Default reminder",
    "description": "Settings > Calendar: row and dialog title for the reminder of timed events whose calendar has none. src/renderer/calendar-settings.js"
  },
  "settings.calendar.reminderHelp": {
    "message": "For timed events whose calendar doesn't set reminders, such as CalDAV. Google events keep their own.",
    "description": "Settings > Calendar: help under Default reminder. src/renderer/calendar-settings.js"
  },
  "settings.calendar.reminderAtStart": {
    "message": "At the start",
    "description": "Default reminder option: when the event starts. src/renderer/calendar-settings.js"
  },
  "settings.calendar.reminderBefore": {
    "message": "{length} before",
    "description": "Default reminder option; length is a duration such as \"10 min\". src/renderer/calendar-settings.js"
  },
  "settings.calendar.calendars": {
    "message": "Calendars",
    "description": "Settings > Calendar: group heading of the list of calendars. src/renderer/calendar-settings.js"
  },
  "settings.calendar.calendarsHelp": {
    "message": "When a calendar counts toward availability, its busy time is left out of your free time, the free times you insert in mail and your booking page. Hiding a calendar doesn't change this.",
    "description": "Settings > Calendar: text above the calendars. src/renderer/calendar-settings.js"
  },
  "settings.calendar.counts": {
    "message": "Counts toward availability",
    "description": "Settings > Calendar: label beside each calendar's switch. src/renderer/calendar-settings.js"
  },
  "settings.calendar.countsLabel": {
    "message": "{name} counts toward availability",
    "description": "Accessible name of a calendar's switch; name is the calendar's name. src/renderer/calendar-settings.js"
  },
  "settings.calendar.color": {
    "message": "Color of {name}",
    "description": "Tooltip of a calendar's color swatch, which opens the color choice; name is the calendar's name. src/renderer/calendar-settings.js"
  },
  "settings.calendar.colorTitle": {
    "message": "Color of {name}",
    "description": "Title of the dialog with the color swatches; name is the calendar's name. src/renderer/calendar-settings.js"
  },
  "settings.calendar.readOnly": {
    "message": "Read-only",
    "description": "Settings > Calendar: note under a calendar you can't change. src/renderer/calendar-settings.js"
  },
  "settings.calendar.noCalendars": {
    "message": "No calendars yet. Add a Google or CalDAV account, or open the calendar once.",
    "description": "Shown instead of the calendars when there are none. src/renderer/calendar-settings.js, src/renderer/booking-settings.js"
  },
  "settings.calendar.listFailed": {
    "message": "Couldn't read your calendars.",
    "description": "Settings > Calendar: the calendar list couldn't be loaded. src/renderer/calendar-settings.js"
  },
  "settings.calendar.reconnect": {
    "message": "Sign in again in the calendar to see these calendars.",
    "description": "Settings > Calendar: an account that needs to sign in again before its calendars can be read. src/renderer/calendar-settings.js"
  },
  "settings.footer.title": {
    "message": "Booking link under signature",
    "description": "Settings row beside the signature (general and per account): adds a line with the booking page's link under the signature. src/renderer/booking-settings.js"
  },
  "settings.footer.help": {
    "message": "Every new message ends with a line and your booking link.",
    "description": "Settings: help under Booking link under signature. src/renderer/booking-settings.js"
  },
  "settings.footer.needsPage": {
    "message": "Shows once your booking page is on.",
    "description": "Settings: help under Booking link under signature while the booking page is off. src/renderer/booking-settings.js"
  },
  "settings.footer.text": {
    "message": "Text of the booking link",
    "description": "Settings row and dialog for the text of the line with the booking link. src/renderer/booking-settings.js"
  },
  "settings.footer.textDefault": {
    "message": "Default text, in your booking page's language",
    "description": "Value of Text of the booking link while the default text is used. src/renderer/booking-settings.js"
  },
  "settings.footer.textHelp": {
    "message": "{url} is where your link goes. The link shows as its address, so it also works in plain text.",
    "description": "Help under the footer text field; url is the literal text {url}. src/renderer/booking-settings.js"
  },
  "settings.footer.needUrl": {
    "message": "Keep {url} in the text: that's where the link goes.",
    "description": "Error in the footer text dialog; url is the literal text {url}. src/renderer/booking-settings.js"
  },
  "settings.footer.tooLong": {
    "message": "At most {count} characters.",
    "description": "Error in the footer text dialog; count is the limit. src/renderer/booking-settings.js"
  },
  "settings.footer.useDefault": {
    "message": "Use default text",
    "description": "Footer text dialog button that goes back to the default text. src/renderer/booking-settings.js"
  },
  "settings.footer.on": {
    "message": "On",
    "description": "Booking link under signature: on. src/renderer/booking-settings.js"
  },
  "settings.footer.off": {
    "message": "Off",
    "description": "Booking link under signature: off. src/renderer/booking-settings.js"
  },
  "settings.footer.onOwnText": {
    "message": "On, with its own text",
    "description": "Account settings: the account's booking link has a text of its own. src/renderer/booking-settings.js"
  },
  "settings.footer.accountGeneral": {
    "message": "Follows the general setting ({state})",
    "description": "Account settings: the account follows the general booking link setting; state is On or Off. src/renderer/booking-settings.js"
  },
  "settings.footer.choiceGeneral": {
    "message": "Follow the general setting",
    "description": "Account choice for the booking link under the signature. src/renderer/booking-settings.js"
  },
  "settings.footer.choiceOn": {
    "message": "On for this account",
    "description": "Account choice for the booking link under the signature. src/renderer/booking-settings.js"
  },
  "settings.footer.choiceOff": {
    "message": "Off for this account",
    "description": "Account choice for the booking link under the signature. src/renderer/booking-settings.js"
  },
  "settings.footer.choiceText": {
    "message": "On, with its own text…",
    "description": "Account choice that opens the text dialog for this account's booking link. src/renderer/booking-settings.js"
  },
  "settings.booking.title": {
    "message": "Booking page",
    "description": "Title of Settings > Booking page and its row: the page where people book time with you. src/renderer/booking-settings.js"
  },
  "settings.booking.rowOff": {
    "message": "Let people and their AI assistants book time with you",
    "description": "Settings overview: second line of the Booking page row while it is off. src/renderer/booking-settings.js"
  },
  "settings.booking.rowOn": {
    "message": "On · {url}",
    "description": "Settings overview: the booking page is on; url is its address without https://. src/renderer/booking-settings.js"
  },
  "settings.booking.rowSignedOut": {
    "message": "Needs a Rukoo account",
    "description": "Settings overview: the booking page needs a Rukoo account first. src/renderer/booking-settings.js"
  },
  "settings.booking.signedOutIntro": {
    "message": "Your booking page is a link where people, and their AI assistants, pick a time with you. It runs on your Rukoo account, so create one or sign in first. You come back here afterwards.",
    "description": "Booking page while signed out of the Rukoo account. src/renderer/booking-settings.js"
  },
  "settings.booking.inviteNote": {
    "message": "{server} takes new accounts by invite code only.",
    "description": "Booking page while signed out, for a server with invite-only sign-ups; server is its operator or host name. src/renderer/booking-settings.js"
  },
  "settings.booking.createAccount": {
    "message": "Create account",
    "description": "Booking page while signed out: opens the Rukoo account sign-up. src/renderer/booking-settings.js"
  },
  "settings.booking.signIn": {
    "message": "Sign in",
    "description": "Booking page while signed out: opens the Rukoo account sign-in. src/renderer/booking-settings.js"
  },
  "settings.booking.privacyNote": {
    "message": "Turning the page on changes what the account server sees: your open and busy times, your meeting types and each booking as it comes in. It stores guests' details sealed, so only your devices can read them afterwards.",
    "description": "Booking page: what turning it on shares with the account server. src/renderer/booking-settings.js"
  },
  "settings.booking.unsupported": {
    "message": "{server} doesn't support booking pages. Ask whoever runs it, or use a server that does.",
    "description": "Booking page on an account server without booking pages; server is its operator or host name. src/renderer/booking-settings.js"
  },
  "settings.booking.loadFailed": {
    "message": "Couldn't load your booking page: {message}",
    "description": "Booking page: its state couldn't be read; message is the reason. src/renderer/booking-settings.js"
  },
  "settings.booking.setupTitle": {
    "message": "Set up booking page",
    "description": "Title of the booking page setup screen. src/renderer/booking-settings.js"
  },
  "settings.booking.setupIntro": {
    "message": "Set up your page on this screen. You can change all of it later.",
    "description": "Booking page setup: text above the form. src/renderer/booking-settings.js"
  },
  "settings.booking.offIntro": {
    "message": "Your booking page is off. Turn it on again with these settings, or change them first.",
    "description": "Booking page setup when the page was on before. src/renderer/booking-settings.js"
  },
  "settings.booking.handle": {
    "message": "Link",
    "description": "Booking page: the field and row for the last part of the page's address (the handle). src/renderer/booking-settings.js"
  },
  "settings.booking.handleHelp": {
    "message": "3 to 32 lowercase letters, digits or hyphens.",
    "description": "Booking page: what a handle may contain; also its error. src/renderer/booking-settings.js"
  },
  "settings.booking.handleChecking": {
    "message": "Checking…",
    "description": "Booking page setup: the handle is being checked with the server. src/renderer/booking-settings.js"
  },
  "settings.booking.handleAvailable": {
    "message": "Available",
    "description": "Booking page setup: nobody has this handle. src/renderer/booking-settings.js"
  },
  "settings.booking.handleYours": {
    "message": "This one is yours",
    "description": "Booking page setup: the account already holds this handle. src/renderer/booking-settings.js"
  },
  "settings.booking.handleTaken": {
    "message": "Someone else has this one",
    "description": "Booking page setup: another account holds this handle. src/renderer/booking-settings.js"
  },
  "settings.booking.handleReserved": {
    "message": "This name is reserved",
    "description": "Booking page setup: the handle is reserved. src/renderer/booking-settings.js"
  },
  "settings.booking.name": {
    "message": "Your name on the page",
    "description": "Booking page: field and row for the name guests see. src/renderer/booking-settings.js"
  },
  "settings.booking.nameNeeded": {
    "message": "Fill in the name people see.",
    "description": "Booking page: error when the name is empty. src/renderer/booking-settings.js"
  },
  "settings.booking.zone": {
    "message": "Time zone",
    "description": "Booking page: field, row and dialog for the page's time zone. src/renderer/booking-settings.js"
  },
  "settings.booking.hours": {
    "message": "Available hours",
    "description": "Booking page: the hours the page offers times in; also the hours page's title. src/renderer/booking-settings.js"
  },
  "settings.booking.hoursWorking": {
    "message": "Your working hours: {hours}",
    "description": "Booking page: the page follows the working hours; hours is such as \"Mon–Fri, 09:00–17:00\". src/renderer/booking-settings.js"
  },
  "settings.booking.hoursCustom": {
    "message": "Own schedule",
    "description": "Booking page: the page has hours of its own. src/renderer/booking-settings.js"
  },
  "settings.booking.hoursSetupHelp": {
    "message": "You can set other hours after turning the page on.",
    "description": "Booking page setup: help under the hours. src/renderer/booking-settings.js"
  },
  "settings.booking.calendars": {
    "message": "Check these calendars for conflicts",
    "description": "Booking page setup: heading of the calendars whose busy time is never offered. src/renderer/booking-settings.js"
  },
  "settings.booking.calendarsHelp": {
    "message": "Times when you're busy in these calendars are never offered.",
    "description": "Booking page setup: help under the calendar choice. src/renderer/booking-settings.js"
  },
  "settings.booking.destination": {
    "message": "Add bookings to",
    "description": "Booking page: the calendar that bookings are added to. src/renderer/booking-settings.js"
  },
  "settings.booking.invitesFrom": {
    "message": "Guests see invitations from {address}.",
    "description": "Booking page: under Add bookings to; address is that calendar's mail account. src/renderer/booking-settings.js"
  },
  "settings.booking.noInvites": {
    "message": "This calendar sends no invitations. Guests only get the booking page's confirmation.",
    "description": "Booking page: under Add bookings to, for a calendar that can't invite (the demo account). src/renderer/booking-settings.js"
  },
  "settings.booking.noDestination": {
    "message": "No calendar can take bookings yet. Connect a Google calendar, or a CalDAV calendar whose server sends invitations.",
    "description": "Booking page setup: no calendar can be the destination. src/renderer/booking-settings.js"
  },
  "settings.booking.destinationGone": {
    "message": "Not found on this computer",
    "description": "Booking page: the destination calendar isn't on this computer. src/renderer/booking-settings.js"
  },
  "settings.booking.turnOn": {
    "message": "Turn on",
    "description": "Booking page setup: the button that turns the page on. src/renderer/booking-settings.js"
  },
  "settings.booking.turningOn": {
    "message": "Turning on…",
    "description": "Booking page setup: the page is being turned on. src/renderer/booking-settings.js"
  },
  "settings.booking.doneTitle": {
    "message": "Your booking page is on",
    "description": "Booking page: card right after turning it on, with Copy link, Preview page and Add to signature. src/renderer/booking-settings.js"
  },
  "settings.booking.copyLink": {
    "message": "Copy link",
    "description": "Booking page button: copies the page's address. src/renderer/booking-settings.js"
  },
  "settings.booking.preview": {
    "message": "Preview page",
    "description": "Booking page button: opens the page in the browser. src/renderer/booking-settings.js"
  },
  "settings.booking.addToSignature": {
    "message": "Add to signature",
    "description": "Booking page button after turning it on: turns on the booking link under the signature. src/renderer/booking-settings.js"
  },
  "settings.booking.inSignature": {
    "message": "In your signature",
    "description": "Booking page: the booking link is already under the signature. src/renderer/booking-settings.js"
  },
  "settings.booking.addedToSignature": {
    "message": "New messages now end with your booking link",
    "description": "Toast after Add to signature. src/renderer/booking-settings.js"
  },
  "settings.booking.copied": {
    "message": "Link copied",
    "description": "Toast after Copy link. src/renderer/booking-settings.js"
  },
  "settings.booking.yourLink": {
    "message": "Your link",
    "description": "Booking page: group heading of the page's address and its state. src/renderer/booking-settings.js"
  },
  "settings.booking.taking": {
    "message": "Take bookings",
    "description": "Booking page switch that turns the page off. src/renderer/booking-settings.js"
  },
  "settings.booking.takingHelp": {
    "message": "Turning this off deletes your page and its bookings from the server. Events already booked stay in your calendar.",
    "description": "Booking page: help under Take bookings. src/renderer/booking-settings.js"
  },
  "settings.booking.offTitle": {
    "message": "Turn off your booking page?",
    "description": "Booking page: title of the question before turning it off. src/renderer/booking-settings.js"
  },
  "settings.booking.offConfirm": {
    "message": "Your page and its bookings are deleted from the server, and your link stops taking bookings. Events already booked stay in your calendar. Your settings stay, so you can turn the page on again.",
    "description": "Booking page: the question before turning it off. src/renderer/booking-settings.js"
  },
  "settings.booking.offAction": {
    "message": "Turn off",
    "description": "Booking page: the button that confirms turning it off. src/renderer/booking-settings.js"
  },
  "settings.booking.turnedOff": {
    "message": "Your booking page is off",
    "description": "Toast after turning the booking page off. src/renderer/booking-settings.js"
  },
  "settings.booking.syncNow": {
    "message": "Check for bookings now",
    "description": "Booking page row: publishes the open times and fetches new bookings. src/renderer/booking-settings.js"
  },
  "settings.booking.syncing": {
    "message": "Checking…",
    "description": "Booking page: a check is running. src/renderer/booking-settings.js"
  },
  "settings.booking.synced": {
    "message": "Booking page up to date",
    "description": "Toast after Check for bookings now. src/renderer/booking-settings.js"
  },
  "settings.booking.justNow": {
    "message": "just now",
    "description": "Booking page: the ago value of settings.booking.lastUpdated for a publish less than a minute ago, mid-sentence and lowercase. src/renderer/booking-settings.js (ago)"
  },
  "settings.booking.lastUpdated": {
    "message": "Last updated from this computer: {ago}",
    "description": "Booking page: when this computer last published the open times; ago is such as \"3 minutes ago\". src/renderer/booking-settings.js"
  },
  "settings.booking.neverUpdated": {
    "message": "Not published from this computer yet",
    "description": "Booking page: this computer hasn't published yet. src/renderer/booking-settings.js"
  },
  "settings.booking.staleHelp": {
    "message": "Until Rukoo publishes again, new bookings wait for your approval.",
    "description": "Booking page: after Last updated, when that was over a day ago. src/renderer/booking-settings.js"
  },
  "settings.booking.unchecked": {
    "message": "Couldn't check {email} since {time}.",
    "description": "Booking page: an account whose busy time couldn't be read; email is its address, time such as \"Tuesday 3 November, 14:00\". src/renderer/booking-settings.js"
  },
  "settings.booking.otherDevice": {
    "message": "Bookings are handled by {device}",
    "description": "Booking page on a computer that isn't the booking device; device is the other computer's name. src/renderer/booking-settings.js"
  },
  "settings.booking.otherDeviceGone": {
    "message": "Bookings are handled by a computer that has signed out",
    "description": "Booking page: the booking device's session has ended. src/renderer/booking-settings.js"
  },
  "settings.booking.otherDeviceHelp": {
    "message": "That computer publishes your open times and adds bookings to your calendar. Use this one instead if that computer stays off.",
    "description": "Booking page: help under the other computer's name. src/renderer/booking-settings.js"
  },
  "settings.booking.otherDeviceVideo": {
    "message": "Before you use this computer, connect {services} here in Video meetings, or new bookings get no meeting link.",
    "description": "Booking page: warning under the other computer's name when meeting types use video services that aren't connected on this computer. services lists them, such as 'Zoom and Microsoft Teams'. src/renderer/booking-settings.js"
  },
  "settings.booking.takeover": {
    "message": "Use this computer",
    "description": "Booking page button: this computer becomes the one that handles bookings. src/renderer/booking-settings.js"
  },
  "settings.booking.takenOver": {
    "message": "This computer now handles your bookings",
    "description": "Toast after Use this computer. src/renderer/booking-settings.js"
  },
  "settings.booking.requests": {
    "message": "Requests",
    "description": "Booking page: group heading of bookings that wait for approval. src/renderer/booking-settings.js"
  },
  "settings.booking.bookings": {
    "message": "Upcoming bookings",
    "description": "Booking page: group heading of confirmed bookings still to come. src/renderer/booking-settings.js"
  },
  "settings.booking.noBookings": {
    "message": "No bookings yet.",
    "description": "Booking page: no upcoming bookings. src/renderer/booking-settings.js"
  },
  "settings.booking.settingsMissing": {
    "message": "This page's settings haven't reached this computer yet. They show up here after the next sync.",
    "description": "Booking page: the page is on but its settings record isn't on this computer yet (it was set up on another computer), so meeting types, hours and page details can't be changed here yet. Also a toast when one of those rows is clicked then. src/renderer/booking-settings.js"
  },
  "settings.booking.guestUnknown": {
    "message": "Guest details can't be read on this computer",
    "description": "Booking page: a booking whose guest details this computer can't open. src/renderer/booking-settings.js"
  },
  "settings.booking.via": {
    "message": "via {via}",
    "description": "Booking page: the assistant or app the guest booked through, as the guest wrote it. src/renderer/booking-settings.js"
  },
  "settings.booking.customLength": {
    "message": "custom length",
    "description": "Booking page: the host changed the event's length in the calendar. src/renderer/booking-settings.js"
  },
  "settings.booking.expires": {
    "message": "waits for you until {time}",
    "description": "Booking page: a request lapses at time, such as \"Tuesday 3 November, 14:00\". src/renderer/booking-settings.js"
  },
  "settings.booking.accept": {
    "message": "Accept",
    "description": "Booking page button: accepts a request. src/renderer/booking-settings.js"
  },
  "settings.booking.decline": {
    "message": "Decline",
    "description": "Booking page button: declines a request; also confirms it in the dialog. src/renderer/booking-settings.js"
  },
  "settings.booking.cancel": {
    "message": "Cancel booking",
    "description": "Booking page button: cancels a confirmed booking; also confirms it in the dialog. src/renderer/booking-settings.js"
  },
  "settings.booking.keep": {
    "message": "Go back",
    "description": "Decline and cancel dialogs: closes without doing anything. src/renderer/booking-settings.js"
  },
  "settings.booking.declineTitle": {
    "message": "Decline this request?",
    "description": "Title of the decline dialog. src/renderer/booking-settings.js"
  },
  "settings.booking.declineEmail": {
    "message": "Also email this message from {address}",
    "description": "Decline dialog checkbox; address is the destination calendar's mail account. src/renderer/booking-settings.js"
  },
  "settings.booking.message": {
    "message": "Message to the guest (optional)",
    "description": "Decline and cancel dialogs: the host's message, shown on the guest's confirmation page (the page they manage the booking on). src/renderer/booking-settings.js"
  },
  "settings.booking.cancelTitle": {
    "message": "Cancel this booking?",
    "description": "Title of the cancel dialog. src/renderer/booking-settings.js"
  },
  "settings.booking.cancelHelp": {
    "message": "The event leaves your calendar and the guest's booking is cancelled. Your message shows on the guest's confirmation page.",
    "description": "Cancel dialog text. src/renderer/booking-settings.js"
  },
  "settings.booking.cancelNotify": {
    "message": "Send the guest a cancellation from your calendar",
    "description": "Cancel dialog checkbox, on by default. src/renderer/booking-settings.js"
  },
  "settings.booking.accepted": {
    "message": "Request accepted",
    "description": "Toast after Accept. src/renderer/booking-settings.js"
  },
  "settings.booking.declined": {
    "message": "Request declined",
    "description": "Toast after Decline. src/renderer/booking-settings.js"
  },
  "settings.booking.cancelled": {
    "message": "Booking cancelled",
    "description": "Toast after Cancel booking. src/renderer/booking-settings.js"
  },
  "settings.booking.types": {
    "message": "Meeting types",
    "description": "Booking page: group heading of the kinds of meeting people can book. src/renderer/booking-settings.js"
  },
  "settings.booking.typeConfirm": {
    "message": "Needs your approval",
    "description": "Meeting type: bookings wait as requests; in the type's description and as its checkbox. src/renderer/booking-settings.js"
  },
  "settings.booking.typeHidden": {
    "message": "Hidden",
    "description": "Meeting type description: the type isn't listed on the page. src/renderer/booking-settings.js"
  },
  "settings.booking.rowBufferBefore": {
    "message": "{time} free time before",
    "description": "Meeting type's row on the booking page, shown when the type keeps time free before a booking; time is a length such as \"15 mins\". src/renderer/booking-settings.js"
  },
  "settings.booking.rowBufferAfter": {
    "message": "{time} free time after",
    "description": "Meeting type's row on the booking page, shown when the type keeps time free after a booking; time is a length such as \"15 mins\". src/renderer/booking-settings.js"
  },
  "settings.booking.rowBufferBoth": {
    "message": "{time} free time before and after",
    "description": "Meeting type's row on the booking page, when the free time before and after a booking are the same; time is a length such as \"15 mins\". src/renderer/booking-settings.js"
  },
  "settings.booking.rowNotice": {
    "message": "{time} notice",
    "description": "Meeting type's row on the booking page, when its minimum notice isn't the default 4 hours; time is a length such as \"1 day\" or \"30 mins\". src/renderer/booking-settings.js"
  },
  "settings.booking.rowNoNotice": {
    "message": "No minimum notice",
    "description": "Meeting type's row on the booking page, when guests can book it right up to its start. src/renderer/booking-settings.js"
  },
  "settings.booking.rowHorizon": {
    "message": { "one": "Up to {count} day ahead", "other": "Up to {count} days ahead" },
    "description": "Meeting type's row on the booking page, when it can be booked more or fewer days ahead than the default 30; count is the number of days. src/renderer/booking-settings.js"
  },
  "settings.booking.rowInterval": {
    "message": "Start times every {time}",
    "description": "Meeting type's row on the booking page, when the offered start times have their own step; time is a length such as \"15 mins\". src/renderer/booking-settings.js"
  },
  "settings.booking.rowLimit": {
    "message": { "one": "At most {count} a day", "other": "At most {count} a day" },
    "description": "Meeting type's row on the booking page, when the type has a limit of bookings per day; count is the limit. src/renderer/booking-settings.js"
  },
  "settings.booking.addType": {
    "message": "Add meeting type",
    "description": "Booking page row that adds a meeting type (up to 10). src/renderer/booking-settings.js"
  },
  "settings.booking.availability": {
    "message": "Availability",
    "description": "Booking page: group heading of hours, calendars, destination and the daily cap. src/renderer/booking-settings.js"
  },
  "settings.booking.checked": {
    "message": "Calendars checked for conflicts",
    "description": "Booking page row that leads to Settings > Calendar. src/renderer/booking-settings.js"
  },
  "settings.booking.checkedCount": {
    "message": { "one": "{count} calendar", "other": "{count} calendars" },
    "description": "Booking page: how many calendars are checked for conflicts. src/renderer/booking-settings.js"
  },
  "settings.booking.overridesCount": {
    "message": { "one": "{count} date with other hours", "other": "{count} dates with other hours" },
    "description": "Booking page: after the hours, how many dates have hours of their own. src/renderer/booking-settings.js"
  },
  "settings.booking.dailyCap": {
    "message": "Bookings per day",
    "description": "Booking page row and prompt: the most new bookings a day. src/renderer/booking-settings.js"
  },
  "settings.booking.dailyCapHelp": {
    "message": "At most this many new bookings a day, from 1 to 50.",
    "description": "Booking page: help under Bookings per day; also its error. src/renderer/booking-settings.js"
  },
  "settings.booking.page": {
    "message": "Page",
    "description": "Booking page: group heading of the page's name, texts, zone, language and link. src/renderer/booking-settings.js"
  },
  "settings.booking.intro": {
    "message": "Introduction",
    "description": "Booking page row and prompt: a short text at the top of the page. src/renderer/booking-settings.js"
  },
  "settings.booking.contact": {
    "message": "Contact line",
    "description": "Booking page row and prompt: a line shown when no times are open. src/renderer/booking-settings.js"
  },
  "settings.booking.contactHelp": {
    "message": "Shows when no times are open.",
    "description": "Booking page: help under Contact line. src/renderer/booking-settings.js"
  },
  "settings.booking.contactPlaceholder": {
    "message": "No time that works? Reply to my last email.",
    "description": "Placeholder of the contact line prompt. src/renderer/booking-settings.js"
  },
  "settings.booking.pageLanguage": {
    "message": "Page language",
    "description": "Booking page row and dialog: the page's default language. src/renderer/booking-settings.js"
  },
  "settings.booking.pageLanguageHelp": {
    "message": "Also the language of the booking link's text and of booked events' titles.",
    "description": "Booking page: help under Page language. src/renderer/booking-settings.js"
  },
  "settings.booking.saved": {
    "message": "Saved",
    "description": "Toast after a booking page change was saved. src/renderer/booking-settings.js"
  },
  "settings.booking.typeNew": {
    "message": "New meeting type",
    "description": "Title of the meeting type editor for a new type. src/renderer/booking-settings.js"
  },
  "settings.booking.typeTitle": {
    "message": "Title",
    "description": "Meeting type editor: the type's title. src/renderer/booking-settings.js"
  },
  "settings.booking.titleNeeded": {
    "message": "Give the meeting type a title.",
    "description": "Meeting type editor: error when the title is empty. src/renderer/booking-settings.js"
  },
  "settings.booking.typeSlug": {
    "message": "Name in the link",
    "description": "Meeting type editor: the last part of the type's own address. src/renderer/booking-settings.js"
  },
  "settings.booking.slugHelp": {
    "message": "Up to 40 lowercase letters, digits or hyphens, and not a reserved word.",
    "description": "Meeting type editor: error for a name in the link that isn't allowed. src/renderer/booking-settings.js"
  },
  "settings.booking.slugTaken": {
    "message": "Another meeting type already uses this name.",
    "description": "Meeting type editor: error for a name in the link another type has. src/renderer/booking-settings.js"
  },
  "settings.booking.typeLengths": {
    "message": "Available durations",
    "description": "Meeting type editor: heading over the lengths a guest can pick from on the booking page, each a toggle such as 15 min or 1 hr. src/renderer/booking-settings.js"
  },
  "settings.booking.typeLengthsHelp": {
    "message": "Guests pick one on the booking page. With one, the page shows no choice.",
    "description": "Meeting type editor: help under Available durations. src/renderer/booking-settings.js"
  },
  "settings.booking.lengthsNeeded": {
    "message": "Pick at least one duration.",
    "description": "Meeting type editor: error when no duration is selected under Available durations. src/renderer/booking-settings.js"
  },
  "settings.booking.typeDefaultLength": {
    "message": "Default duration",
    "description": "Meeting type editor: which of the available durations is selected when a guest opens the booking page. Shown when more than one is selected. src/renderer/booking-settings.js"
  },
  "settings.booking.typeDefaultLengthHelp": {
    "message": "Selected when a guest opens the page, and the length Insert my free times uses.",
    "description": "Meeting type editor: help under Default duration. Insert my free times is the composer's calendar menu item. src/renderer/booking-settings.js"
  },
  "settings.booking.typeDescription": {
    "message": "Description",
    "description": "Meeting type editor: the type's description on the page. src/renderer/booking-settings.js"
  },
  "settings.booking.location": {
    "message": "Location",
    "description": "Meeting type editor: the kind of location. src/renderer/booking-settings.js"
  },
  "settings.booking.kindMeet": {
    "message": "Google Meet",
    "description": "Location kind: a Google Meet video call. src/renderer/booking-settings.js"
  },
  "settings.booking.kindPhone": {
    "message": "Phone call",
    "description": "Location kind: a phone call. src/renderer/booking-settings.js"
  },
  "settings.booking.kindInPerson": {
    "message": "In person",
    "description": "Location kind: meeting in person. src/renderer/booking-settings.js"
  },
  "settings.booking.kindLink": {
    "message": "Meeting link",
    "description": "Location kind: a link to another video service. src/renderer/booking-settings.js"
  },
  "settings.booking.kindNone": {
    "message": "No location",
    "description": "Location kind: none. src/renderer/booking-settings.js"
  },
  "settings.booking.locationValue": {
    "message": "Details",
    "description": "Meeting type editor: the phone number, address or link of the location. src/renderer/booking-settings.js"
  },
  "settings.booking.valuePhone": {
    "message": "Phone number",
    "description": "Placeholder of the location details for a phone call. src/renderer/booking-settings.js"
  },
  "settings.booking.valueAddress": {
    "message": "Address",
    "description": "Placeholder of the location details for in person. src/renderer/booking-settings.js"
  },
  "settings.booking.valueLink": {
    "message": "Link to the meeting",
    "description": "Placeholder of the location details for a meeting link. src/renderer/booking-settings.js"
  },
  "settings.booking.locationPublic": {
    "message": "Show these details on the booking page",
    "description": "Meeting type editor checkbox: publishes the location details. src/renderer/booking-settings.js"
  },
  "settings.booking.locationPrivateHelp": {
    "message": "Otherwise only the guest's invitation has them.",
    "description": "Meeting type editor: help under the checkbox. src/renderer/booking-settings.js"
  },
  "settings.booking.locationLabel": {
    "message": "Label on the page",
    "description": "Meeting type editor: what the page says about the location. src/renderer/booking-settings.js"
  },
  "settings.booking.locationLabelPlaceholder": {
    "message": "Such as Phone call, or In person, Amsterdam",
    "description": "Placeholder of the location label. src/renderer/booking-settings.js"
  },
  "settings.booking.meetUnavailable": {
    "message": "Google Meet needs a Google calendar that allows Meet under Add bookings to.",
    "description": "Meeting type editor: why Google Meet can't be chosen. src/renderer/booking-settings.js"
  },
  "settings.booking.videoRenewOption": {
    "message": "{service} (needs renewing)",
    "description": "Meeting type editor: a Location option for a video service whose connection stopped working on this computer. service is Zoom or Microsoft Teams. src/renderer/booking-settings.js"
  },
  "settings.booking.videoOffOption": {
    "message": "{service} (not connected here)",
    "description": "Meeting type editor: the Location option of a type whose video service isn't connected on this computer. service is Zoom, Microsoft Teams or Jitsi Meet. src/renderer/booking-settings.js"
  },
  "settings.booking.videoHelp": {
    "message": "Each booking gets its own {service} link in the guest's invitation. The page shows only the label.",
    "description": "Meeting type editor: help under Location when a video service is chosen. service is Zoom, Microsoft Teams or Jitsi Meet. src/renderer/booking-settings.js"
  },
  "settings.booking.videoOtherDevice": {
    "message": "Video services are chosen on the computer that handles your bookings, where they are connected.",
    "description": "Meeting type editor: help under Location on a computer that doesn't handle bookings, so it offers no video services. src/renderer/booking-settings.js"
  },
  "settings.booking.videoNone": {
    "message": "To give each booking its own meeting link, connect Zoom, Microsoft Teams or Jitsi Meet in Settings → Video meetings.",
    "description": "Meeting type editor: help under Location while no video service is connected on this computer. src/renderer/booking-settings.js"
  },
  "settings.booking.linkHttps": {
    "message": "A meeting link starts with https://.",
    "description": "Meeting type editor: error for a link that isn't https. src/renderer/booking-settings.js"
  },
  "settings.booking.when": {
    "message": "When guests can book",
    "description": "Meeting type editor: heading of the section, open when the editor opens, with free time before and after, minimum notice, how far ahead, start times and the limit per day. src/renderer/booking-settings.js"
  },
  "settings.booking.more": {
    "message": "More options",
    "description": "Meeting type editor: folded section with Needs your approval and Hide from the page. src/renderer/booking-settings.js"
  },
  "settings.booking.bufferBefore": {
    "message": "Free time before",
    "description": "Meeting type editor: buffer before a booking. src/renderer/booking-settings.js"
  },
  "settings.booking.bufferAfter": {
    "message": "Free time after",
    "description": "Meeting type editor: buffer after a booking. src/renderer/booking-settings.js"
  },
  "settings.booking.bufferHelp": {
    "message": "Keeps time free around each booking, so guests can't book right next to your other meetings.",
    "description": "Meeting type editor: help under Free time before and Free time after, for hosts who look for buffers. src/renderer/booking-settings.js"
  },
  "settings.booking.minNotice": {
    "message": "Minimum notice",
    "description": "Meeting type editor: how long before the start a booking can be made. src/renderer/booking-settings.js"
  },
  "settings.booking.noNotice": {
    "message": "No minimum",
    "description": "Minimum notice option: none. src/renderer/booking-settings.js"
  },
  "settings.booking.horizon": {
    "message": "How far ahead",
    "description": "Meeting type editor: how many days ahead can be booked. src/renderer/booking-settings.js"
  },
  "settings.booking.slotInterval": {
    "message": "Start times every",
    "description": "Meeting type editor: the step between offered start times. src/renderer/booking-settings.js"
  },
  "settings.booking.slotSame": {
    "message": "Same as the length",
    "description": "Start times every option: the meeting's length. src/renderer/booking-settings.js"
  },
  "settings.booking.dailyLimit": {
    "message": "At most per day",
    "description": "Meeting type editor: the most bookings of this type a day. src/renderer/booking-settings.js"
  },
  "settings.booking.noLimit": {
    "message": "No limit",
    "description": "At most per day option: none. src/renderer/booking-settings.js"
  },
  "settings.booking.days": {
    "message": { "one": "{count} day", "other": "{count} days" },
    "description": "A number of days, in the meeting type editor. src/renderer/booking-settings.js"
  },
  "settings.booking.perDay": {
    "message": { "one": "{count} a day", "other": "{count} a day" },
    "description": "At most per day option; count is the number. src/renderer/booking-settings.js"
  },
  "settings.booking.confirmHelp": {
    "message": "Bookings of this type wait as requests until you accept them.",
    "description": "Meeting type editor: help under Needs your approval. src/renderer/booking-settings.js"
  },
  "settings.booking.hide": {
    "message": "Hide from the page",
    "description": "Meeting type editor checkbox. src/renderer/booking-settings.js"
  },
  "settings.booking.hideHelp": {
    "message": "Only people with its own link can book it.",
    "description": "Meeting type editor: help under Hide from the page. src/renderer/booking-settings.js"
  },
  "settings.booking.deleteType": {
    "message": "Delete meeting type",
    "description": "Meeting type editor button and question title. src/renderer/booking-settings.js"
  },
  "settings.booking.deleteTypeConfirm": {
    "message": "Delete {title}? Its link stops working.",
    "description": "Meeting type editor question; title is the type's title. src/renderer/booking-settings.js"
  },
  "settings.booking.hoursCustomToggle": {
    "message": "Use different hours for my booking page",
    "description": "Hours page switch. src/renderer/booking-settings.js"
  },
  "settings.booking.hoursCustomHelp": {
    "message": "The page offers times only in the hours below.",
    "description": "Hours page: help while the page has its own hours. src/renderer/booking-settings.js"
  },
  "settings.booking.hoursWorkingHelp": {
    "message": "Off: the page follows your working hours from Settings > Calendar: {hours}.",
    "description": "Hours page: help while the page follows the working hours; hours is such as \"Mon–Fri, 09:00–17:00\". src/renderer/booking-settings.js"
  },
  "settings.booking.weekly": {
    "message": "Weekly hours",
    "description": "Hours page: group heading of the hours per weekday. src/renderer/booking-settings.js"
  },
  "settings.booking.unavailableDay": {
    "message": "Unavailable",
    "description": "Hours page: a weekday without hours. src/renderer/booking-settings.js"
  },
  "settings.booking.addRange": {
    "message": "Add hours on {day}",
    "description": "Hours page button; day is a weekday's name. src/renderer/booking-settings.js"
  },
  "settings.booking.removeRange": {
    "message": "Remove these hours",
    "description": "Hours page button beside a range. src/renderer/booking-settings.js"
  },
  "settings.booking.rangeFrom": {
    "message": "{day}: from",
    "description": "Accessible name of a range's start field; day is a weekday or date. src/renderer/booking-settings.js"
  },
  "settings.booking.rangeTo": {
    "message": "{day}: to",
    "description": "Accessible name of a range's end field; day is a weekday or date. src/renderer/booking-settings.js"
  },
  "settings.booking.rangeInvalid": {
    "message": "Each range has to end after it starts, and ranges on one day can't overlap. At most six a day.",
    "description": "Hours page error. src/renderer/booking-settings.js"
  },
  "settings.booking.overrides": {
    "message": "Dates with other hours",
    "description": "Hours page: group heading of the date overrides. src/renderer/booking-settings.js"
  },
  "settings.booking.overridesHelp": {
    "message": "Block a day, or set other hours for one date.",
    "description": "Hours page: text above the date overrides. src/renderer/booking-settings.js"
  },
  "settings.booking.addOverride": {
    "message": "Add date",
    "description": "Hours page row and dialog title. src/renderer/booking-settings.js"
  },
  "settings.booking.overrideDate": {
    "message": "Date",
    "description": "Add date dialog: the date. src/renderer/booking-settings.js"
  },
  "settings.booking.overrideDateNeeded": {
    "message": "Pick a date from today on.",
    "description": "Add date dialog error. src/renderer/booking-settings.js"
  },
  "settings.booking.overrideBlock": {
    "message": "Block the whole day",
    "description": "Add date dialog option. src/renderer/booking-settings.js"
  },
  "settings.booking.overrideHours": {
    "message": "Other hours",
    "description": "Add date dialog option. src/renderer/booking-settings.js"
  },
  "settings.booking.blockedDay": {
    "message": "Blocked",
    "description": "Hours page: a date without any hours. src/renderer/booking-settings.js"
  },
  "settings.booking.removeOverride": {
    "message": "Remove {date}",
    "description": "Hours page button; date is such as \"Tuesday 3 November\". src/renderer/booking-settings.js"
  },
  "settings.booking.privacyTitle": {
    "message": "With your booking page on",
    "description": "Settings > Rukoo account > What's stored and shared: heading of what the server sees for a booking page. src/renderer/rukoo-account.js (privacyPage)"
  },
  "settings.booking.privacySee": {
    "message": "Your open and busy times, your meeting types and each booking as it comes in. Guests' details are stored sealed, so only your devices can read them afterwards.",
    "description": "Settings > Rukoo account > What's stored and shared: what the server sees while the booking page is on. src/renderer/rukoo-account.js (privacyPage)"
  },
  "settings.spam.add": {
    "message": "Add spam address",
    "description": "settings > spam. src/renderer/settings.js (openSettings)"
  },
  "settings.spam.empty": {
    "message": "No spam addresses",
    "description": "settings > spam. src/renderer/settings.js (render)"
  },
  "settings.sync.fifteenMinutes": {
    "message": "Every 15 minutes",
    "description": "settings > sync. src/renderer/settings.js"
  },
  "settings.sync.fiveMinutes": {
    "message": "Every 5 minutes",
    "description": "settings > sync. src/renderer/settings.js"
  },
  "settings.sync.hour": {
    "message": "Every hour",
    "description": "settings > sync. src/renderer/settings.js"
  },
  "settings.sync.manual": {
    "message": "Manual",
    "description": "settings > sync. src/renderer/settings.js"
  },
  "settings.sync.minute": {
    "message": "Every minute",
    "description": "settings > sync. src/renderer/settings.js"
  },
  "settings.sync.thirtyMinutes": {
    "message": "Every 30 minutes",
    "description": "settings > sync. src/renderer/settings.js"
  },
  "settings.theme.dark": {
    "message": "Dark",
    "description": "settings > theme. src/renderer/settings.js"
  },
  "settings.theme.light": {
    "message": "Light",
    "description": "settings > theme. src/renderer/settings.js"
  },
  "settings.theme.system": {
    "message": "Follow system setting",
    "description": "settings > theme. src/renderer/settings.js"
  },
  "settings.title": {
    "message": "Email settings",
    "description": "settings. src/renderer/settings.js (render)"
  },
  "settings.title.short": {
    "message": "Settings",
    "description": "settings > title. src/renderer/app.js (renderSidebar); src/renderer/app.js (listAction)"
  },
  "settings.vip.add": {
    "message": "Add VIP",
    "description": "settings > vip. src/renderer/settings.js (openSettings)"
  },
  "settings.vip.empty": {
    "message": "No VIPs yet. Add a sender from the More actions menu in an email.",
    "description": "settings > vip. src/renderer/settings.js (render)"
  },
  "setup.account.added": {
    "message": "{email} has been added",
    "description": "setup > account. src/renderer/setup.js (done)"
  },
  "setup.appPassword.create": {
    "message": "Create app password",
    "description": "Setup form for Google and Yahoo: link that opens the provider's app password page in the browser. Shown in the note above the password field and under a refused sign-in. src/renderer/setup.js (showLogin)"
  },
  "setup.demo.action": {
    "message": "Try a demo account first",
    "description": "setup > demo. src/renderer/setup.js (showGrid)"
  },
  "setup.demo.name": {
    "message": "Demo User",
    "description": "setup > demo. src/main/engine.js"
  },
  "setup.fields.appPasswordHint": {
    "message": "App password",
    "description": "Setup form for Google and Yahoo: placeholder in the password field, which only takes an app password there. src/renderer/setup.js (showLogin)"
  },
  "setup.fields.email": {
    "message": "Email address",
    "description": "setup > fields. src/renderer/setup.js (showLogin)"
  },
  "setup.fields.emailExample": {
    "message": "name@example.com",
    "description": "setup > fields. src/renderer/settings.js (openSettings); src/renderer/setup.js (showLogin)"
  },
  "setup.fields.hidePassword": {
    "message": "Hide password",
    "description": "Login form password visibility toggle when the password is visible."
  },
  "setup.fields.imap": {
    "message": "IMAP server",
    "description": "setup > fields. src/renderer/settings.js (render); src/renderer/setup.js (showLogin)"
  },
  "setup.fields.password": {
    "message": "Password",
    "description": "setup > fields. src/renderer/settings.js (render); src/renderer/setup.js (showLogin)"
  },
  "setup.fields.passwordHint": {
    "message": "Password or app password",
    "description": "setup > fields. src/renderer/setup.js (showLogin)"
  },
  "setup.fields.port": {
    "message": "Port",
    "description": "setup > fields. src/renderer/settings.js (render); src/renderer/setup.js (showLogin)"
  },
  "setup.fields.showPassword": {
    "message": "Show password",
    "description": "setup > fields. src/renderer/setup.js (showLogin)"
  },
  "setup.fields.smtp": {
    "message": "SMTP server",
    "description": "setup > fields. src/renderer/settings.js (render); src/renderer/setup.js (showLogin)"
  },
  "setup.fields.username": {
    "message": "Username",
    "description": "setup > fields. src/renderer/settings.js (render); src/renderer/setup.js (showLogin)"
  },
  "setup.fields.usernameHint": {
    "message": "Usually your email address",
    "description": "setup > fields. src/renderer/setup.js (showLogin)"
  },
  "setup.google.action": {
    "message": "Sign in with Google",
    "description": "setup > google. src/renderer/setup.js (showGoogle)"
  },
  "setup.google.cancelled": {
    "message": "Sign-in cancelled",
    "description": "setup > google. src/main/google.js"
  },
  "setup.google.closeTab": {
    "message": "You can close this tab.",
    "description": "setup > google. src/main/google.js"
  },
  "setup.google.failed": {
    "message": "Sign-in failed",
    "description": "setup > google. src/main/google.js"
  },
  "setup.google.help": {
    "message": "Your browser will open Google's sign-in page. Choose your account and allow Rukoo Mail to access Gmail. You will then return here automatically.",
    "description": "setup > google. src/renderer/setup.js (showGoogle)"
  },
  "setup.google.import": {
    "message": "Import Google OAuth client",
    "description": "setup > google. src/renderer/setup.js (showGrid); src/main/main.js"
  },
  "setup.google.invalidRequest": {
    "message": "Invalid request. Start sign-in again from Rukoo Mail.",
    "description": "setup > google. src/main/google.js"
  },
  "setup.google.passwordMode": {
    "message": "Use an app password",
    "description": "setup > google. src/renderer/setup.js (showGoogle)"
  },
  "setup.google.ready": {
    "message": "Google sign-in is ready",
    "description": "setup > google. src/renderer/setup.js (openSetup)"
  },
  "setup.google.return": {
    "message": "You can close this tab and return to Rukoo Mail.",
    "description": "setup > google. src/main/google.js"
  },
  "setup.google.signedIn": {
    "message": "Signed in to Google",
    "description": "setup > google. src/renderer/settings.js (openSettings)"
  },
  "setup.google.success": {
    "message": "You are signed in",
    "description": "setup > google. src/main/google.js"
  },
  "setup.google.title": {
    "message": "Sign in to Google",
    "description": "setup > google. src/renderer/setup.js (showGoogle)"
  },
  "setup.google.unavailable": {
    "message": "Browser sign-in isn't available: this copy of Rukoo has no Google OAuth client.",
    "description": "Google password form when Rukoo has no Google OAuth client: says why it shows the password form instead of browser sign-in. Followed by the Import Google OAuth client button. src/renderer/setup.js (showLogin)"
  },
  "setup.google.wait": {
    "message": "Waiting for sign-in in your browser...",
    "description": "setup > google. src/renderer/setup.js (showGoogle)"
  },
  "setup.google.waitToast": {
    "message": "Sign in through your browser...",
    "description": "setup > google. src/renderer/settings.js (openSettings)"
  },
  "setup.language.label": {
    "message": "Language",
    "description": "First-run provider picker: interface language selection."
  },
  "setup.manual.action": {
    "message": "Manual setup",
    "description": "setup > manual. Shared interface label."
  },
  "setup.providers.exchange.note": {
    "message": "Enter the IMAP and SMTP servers for your Exchange environment.",
    "description": "setup > providers > exchange. src/main/providers.js"
  },
  "setup.providers.google.note": {
    "message": "Gmail only accepts an app password here, not your normal Google password. Create one in your Google account and enter it below. IMAP must be enabled in Gmail.",
    "description": "Google password form: the note above the password field, followed by the Create app password link. src/main/providers.js; src/renderer/setup.js (showLogin)"
  },
  "setup.providers.office365.note": {
    "message": "Your administrator must enable IMAP and SMTP AUTH for your mailbox.",
    "description": "setup > providers > office365. src/main/providers.js"
  },
  "setup.providers.other.label": {
    "message": "Other",
    "description": "setup > providers > other. src/main/providers.js"
  },
  "setup.providers.other.note": {
    "message": "We suggest servers based on your domain. Adjust them if needed.",
    "description": "setup > providers > other. src/main/providers.js"
  },
  "setup.providers.outlook.note": {
    "message": "Only works if your account allows IMAP with a password or app password.",
    "description": "setup > providers > outlook. src/main/providers.js"
  },
  "setup.providers.yahoo.note": {
    "message": "Yahoo Mail only accepts an app password here, not your normal Yahoo password. Create one under Account security and enter it below.",
    "description": "Yahoo password form: the note above the password field, followed by the Create app password link to Yahoo's Account security page. src/main/providers.js; src/renderer/setup.js (showLogin)"
  },
  "setup.signIn.action": {
    "message": "Sign in",
    "description": "setup > signIn. src/renderer/setup.js (showLogin); src/renderer/setup.js (openSetup)"
  },
  "setup.signIn.pending": {
    "message": "Signing in...",
    "description": "setup > signIn. Setup login form: progress label while connecting to the account."
  },
  "setup.signIn.title": {
    "message": "Sign in to {provider}",
    "description": "setup > signIn. Provider login form heading; provider is a provider display name."
  },
  "errors.credentials.linuxKeyring": {
    "message": "Rukoo cannot securely save credentials. Install and unlock GNOME Keyring or KWallet, then restart Rukoo and try again.",
    "description": "src/main/secrets.js: account or agent credential save error when Linux has no secure system keyring. No parameters."
  },
  "settings.general.mailtoMacHelp": {
    "message": "Open mailto: links from your browser and other apps in Rukoo Mail. macOS may ask you to confirm the change.",
    "description": "macOS email-link integration in Settings > General. src/renderer/settings.js and src/main/main.js."
  },
  "settings.general.mailtoMacUnavailable": {
    "message": "Move Rukoo Mail to Applications and open it there to use it for email links.",
    "description": "macOS email-link integration in Settings > General. src/renderer/settings.js and src/main/main.js."
  },
  "settings.general.mailtoMacFailed": {
    "message": "Could not set the default email app. Open Apple Mail, choose Mail > Settings > General, then select Rukoo Mail as the default email reader.",
    "description": "macOS email-link integration in Settings > General. src/renderer/settings.js and src/main/main.js."
  },
  "errors.credentials.macKeychain": {
    "message": "Rukoo cannot securely save credentials. Unlock your login keychain in Keychain Access, then restart Rukoo and try again.",
    "description": "macOS credential storage error. src/main/secrets.js."
  },
  "native.menu.view": {"message": "View", "description": "Native macOS View menu, src/main/desktop.js (macMenu)."},
  "native.menu.zoomIn": {"message": "Zoom in", "description": "Native macOS View menu, src/main/desktop.js (macMenu)."},
  "native.menu.zoomOut": {"message": "Zoom out", "description": "Native macOS View menu, src/main/desktop.js (macMenu)."},
  "native.menu.actualSize": {"message": "Actual size", "description": "Native macOS View menu, src/main/desktop.js (macMenu)."},
  "native.menu.edit": {
    "message": "Edit",
    "description": "macOS native Edit menu command. src/main/main.js."
  },
  "native.menu.undo": {
    "message": "Undo",
    "description": "macOS native Edit menu command. src/main/main.js."
  },
  "native.menu.redo": {
    "message": "Redo",
    "description": "macOS native Edit menu command. src/main/main.js."
  },
  "native.menu.selectAll": {
    "message": "Select all",
    "description": "macOS native Edit menu command. src/main/main.js."
  },
  "settings.general.macMenuBarIcon": {
    "message": "Show menu bar icon",
    "description": "macOS Settings > General: optional menu bar icon and minimize behavior. src/renderer/settings.js."
  },
  "settings.general.macMenuBarIconHelp": {
    "message": "Add Rukoo to the menu bar. Closing the window always keeps Rukoo running; reopen it from the Dock. Press ⌘+Q to quit. Turning this off also turns off minimizing to the menu bar.",
    "description": "macOS Settings > General: optional menu bar icon and minimize behavior. src/renderer/settings.js."
  },
  "settings.general.macMinimize": {
    "message": "Minimize to menu bar",
    "description": "macOS Settings > General: optional menu bar icon and minimize behavior. src/renderer/settings.js."
  },
  "settings.general.macMinimizeHelp": {
    "message": "Hide the minimized window and show a menu bar icon. Reopen Rukoo from that icon or the Dock.",
    "description": "macOS Settings > General: optional menu bar icon and minimize behavior. src/renderer/settings.js."
  },
  "setup.title": {
    "message": "Set up email",
    "description": "setup. src/renderer/setup.js (showGrid)"
  },
  "account.errors.invalid": {
    "message": "The server didn't accept the request. Update Rukoo and try again.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers invalid or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.unauthorized": {
    "message": "The email address, password or recovery key is wrong.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers unauthorized or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.wrong_password": {
    "message": "That password is wrong.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers wrong_password or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.locked": {
    "message": "Too many attempts. Try again in 15 minutes.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers locked or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.rate_limited": {
    "message": "Too many requests from this network. Wait a minute and try again.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers rate_limited or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.exists": {
    "message": "There is already an account with this email address. Sign in instead.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers exists or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.signups_closed": {
    "message": "This server doesn't take new accounts.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers signups_closed or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.invite_required": {
    "message": "This server needs a valid invite code.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers invite_required or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.quota": {
    "message": "Your account has reached this server's limit.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers quota or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.too_large": {
    "message": "Something is too large for this server to store.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers too_large or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.not_configured": {
    "message": "This server isn't fully set up yet. Its operator needs to finish the setup.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers not_configured or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.network": {
    "message": "Can't reach the server. Check your connection.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers network or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.server": {
    "message": "The server ran into a problem. Try again later.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers server or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.invalid_email": {
    "message": "Enter a valid email address.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers invalid_email or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.weak_password": {
    "message": "Use a password of at least 12 characters.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers weak_password or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.recovery_key": {
    "message": "That isn't a recovery key. A recovery key has 32 letters and digits in groups of four.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers recovery_key or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.weak_kdf": {
    "message": "The server asked for weaker password protection than Rukoo allows, so Rukoo didn't send your password's key.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers weak_kdf or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.no_server": {
    "message": "Enter the address of a Rukoo account server, starting with https://.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers no_server or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.protocol": {
    "message": "This isn't a Rukoo account server, or it needs a newer version of Rukoo.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers protocol or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.signed_in": {
    "message": "Sign out of your Rukoo account first.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers signed_in or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.signed_out": {
    "message": "You were signed out because your password changed or this device was removed. Sign in again.",
    "description": "Rukoo account error, shown in Settings > Rukoo account when the account server answers signed_out or the input is refused. src/main/account/index.js (fail)."
  },
  "account.errors.handle_taken": {
    "message": "Another account uses this address. Pick another one.",
    "description": "Rukoo account error from the booking routes: the booking page address (handle) belongs to another account. Settings > Booking page. src/main/account/index.js (fail)."
  },
  "account.errors.handle_reserved": {
    "message": "That address is reserved. Pick another one.",
    "description": "Rukoo account error from the booking routes: the booking page address is a reserved word or still reserved for its last owner. src/main/account/index.js (fail)."
  },
  "account.errors.handle_lost": {
    "message": "Your booking page's address belongs to another account now. Turn the page on with a new address.",
    "description": "Rukoo account error from the booking routes: this account no longer owns its booking page's address. src/main/account/index.js (fail)."
  },
  "account.errors.handle_changes": {
    "message": "You changed your booking page's address 3 times in the last 30 days. Try again later.",
    "description": "Rukoo account error from the booking routes: a booking page address can change at most 3 times in 30 days. src/main/account/index.js (fail)."
  },
  "account.errors.has_bookings": {
    "message": "The address can't change while bookings are coming up.",
    "description": "Rukoo account error from the booking routes: renaming the booking page is refused while it has future bookings. src/main/account/index.js (fail)."
  },
  "account.errors.other_device": {
    "message": "Another computer handles your bookings.",
    "description": "Rukoo account error from the booking routes: only the booking device may publish or act on bookings. src/main/account/index.js (fail)."
  },
  "account.errors.not_found": {
    "message": "It isn't on the server any more.",
    "description": "Rukoo account error: the server answered not_found, for a booking or a device that no longer exists. src/main/account/index.js (fail)."
  },
  "account.errors.slot_taken": {
    "message": "That time isn't free.",
    "description": "Rukoo account error from the booking routes: the time asked for isn't an open slot. src/main/account/index.js (fail)."
  },
  "account.errors.stale_version": {
    "message": "The booking changed in the meantime. Rukoo now shows it as it is.",
    "description": "Rukoo account error from the booking routes: the booking changed since this computer last read it; Rukoo reads it again. src/main/account/index.js (fail)."
  },
  "account.errors.not_active": {
    "message": "This booking is over, so it can't be cancelled.",
    "description": "Rukoo account error from the booking routes: the host tried to cancel a booking whose meeting has ended. src/main/account/index.js (fail)."
  },
  "booking.errors.unsupported": {
    "message": "This server doesn't support booking pages.",
    "description": "Booking page error: the Rukoo account server doesn't list booking in its features. src/main/booking/index.js."
  },
  "booking.errors.signedOut": {
    "message": "Sign in to your Rukoo account to use a booking page.",
    "description": "Booking page error: a booking action needs a signed-in Rukoo account. src/main/booking/index.js."
  },
  "booking.errors.noDestination": {
    "message": "Choose a calendar that can take bookings: a Google calendar you can edit, or a CalDAV calendar whose server sends invitations.",
    "description": "Booking page error: the calendar chosen for bookings can't take them. Settings > Booking page setup. src/main/booking/index.js."
  },
  "booking.errors.destinationGone": {
    "message": "The calendar for bookings isn't on this computer any more. Choose another one.",
    "description": "Booking page error: the destination calendar can't be found or written on this computer. src/main/booking/index.js."
  },
  "booking.errors.calendarUnreadable": {
    "message": "Rukoo couldn't read the calendar for bookings. Check the connection and try again.",
    "description": "Booking page error: the destination calendar couldn't be read, so Rukoo can't tell whether a booked event is still there (for example when cancelling a booking). src/main/booking/index.js (bookingEvents, findEvent)."
  },
  "booking.errors.noKey": {
    "message": "Rukoo couldn't set up the key for your booking page. Check your connection and try again.",
    "description": "Booking page error: the key pair guest details are sealed to couldn't be made or confirmed with the account server. src/main/booking/index.js (ensureKey)."
  },
  "booking.errors.keyMismatch": {
    "message": "The key for your booking page on this computer is damaged. Sync your Rukoo account and try again.",
    "description": "Booking page error: the stored key pair doesn't hold together. src/main/booking/index.js (ensureKey)."
  },
  "booking.errors.tooLarge": {
    "message": "Your booking page settings are too long. Shorten the descriptions or remove a meeting type.",
    "description": "Booking page error: the settings record is over 12 KiB. Settings > Booking page. src/main/booking/rules.js (checkSettings)."
  },
  "booking.errors.invalidHandle": {
    "message": "Use 3 to 32 lowercase letters, digits and single hyphens, without a hyphen at the start or end.",
    "description": "Booking page error next to the address (handle) field: the handle doesn't follow the rules. src/main/booking/rules.js."
  },
  "booking.errors.reservedHandle": {
    "message": "That address is reserved. Pick another one.",
    "description": "Booking page error next to the address (handle) field: the handle is a reserved word such as api or meet. src/main/booking/rules.js."
  },
  "booking.errors.invalidSlug": {
    "message": "A link name uses 1 to 40 lowercase letters, digits and single hyphens.",
    "description": "Booking page error next to a meeting type's link name (slug). src/main/booking/rules.js."
  },
  "booking.errors.reservedSlug": {
    "message": "That link name is reserved. Pick another one.",
    "description": "Booking page error next to a meeting type's link name: the slug is a reserved word such as api or ics. src/main/booking/rules.js."
  },
  "booking.errors.duplicateSlug": {
    "message": "Two meeting types have the same link name.",
    "description": "Booking page error: two meeting types share a slug. src/main/booking/rules.js."
  },
  "booking.errors.invalidType": {
    "message": "Check the meeting type: one of its fields isn't valid.",
    "description": "Booking page error: a meeting type field is out of range (length, buffers, notice, window, interval, limit, location). src/main/booking/rules.js."
  },
  "booking.errors.invalidHours": {
    "message": "Check the hours: each range ends after it starts, ranges can't overlap, and a day has at most 6.",
    "description": "Booking page error: the booking hours or a date's hours don't make valid ranges. src/main/booking/rules.js."
  },
  "booking.errors.noMeet": {
    "message": "Google Meet needs a Google calendar that allows it as the calendar for bookings.",
    "description": "Booking page error: a meeting type uses Google Meet but the destination calendar can't add it. src/main/booking/rules.js."
  },
  "booking.errors.videoElsewhere": {
    "message": "Choose a video service on the computer that handles your bookings, where it is connected.",
    "description": "Booking page error: a meeting type's location was set to Zoom, Microsoft Teams or Jitsi Meet on a computer that doesn't handle bookings. src/main/booking/index.js (save)."
  },
  "booking.errors.notBookingDevice": {
    "message": "Another computer handles your bookings. Use this computer for bookings first.",
    "description": "Booking page error: only the booking device can publish, rename the page or act on bookings. src/main/booking/index.js."
  },
  "booking.errors.deleteFirst": {
    "message": "Rukoo couldn't turn off your booking page, so your account wasn't deleted. Check your connection and try again.",
    "description": "Settings > Rukoo account > Delete account: deleting stops because the booking page couldn't be turned off first. src/main/booking/index.js (disableForDelete)."
  },
  "booking.errors.invalidName": {
    "message": "Enter a name of up to 100 characters.",
    "description": "Booking page error next to the display name field. src/main/booking/rules.js."
  },
  "booking.errors.invalidIntro": {
    "message": "The introduction can be at most 1000 characters.",
    "description": "Booking page error next to the introduction field. src/main/booking/rules.js."
  },
  "booking.errors.invalidContact": {
    "message": "The contact line can be at most 200 characters on one line.",
    "description": "Booking page error next to the contact line shown when no times are open. src/main/booking/rules.js."
  },
  "booking.errors.invalidZone": {
    "message": "Choose a time zone from the list.",
    "description": "Booking page error next to the time zone field. src/main/booking/rules.js."
  },
  "booking.errors.invalidCap": {
    "message": "The daily limit is a number from 1 to 50.",
    "description": "Booking page error next to the daily booking limit. src/main/booking/rules.js."
  },
  "booking.errors.invalidOverrides": {
    "message": "Check the dates with other hours: at most 60 dates with up to 6 ranges each.",
    "description": "Booking page error: the date overrides (blocked days or other hours) don't fit the limits. src/main/booking/rules.js."
  },
  "booking.errors.invalidMessage": {
    "message": "The message can be at most 500 characters.",
    "description": "Booking page error: the host's message to a guest when declining or cancelling is too long. src/main/booking/index.js (action)."
  },
  "booking.errors.notFound": {
    "message": "This booking isn't there any more.",
    "description": "Booking page error: an action names a booking or notice this computer doesn't have, or one that can't take it. src/main/booking/index.js (action)."
  },
  "booking.errors.notOn": {
    "message": "Your booking page is off.",
    "description": "Booking page error: an action that needs the page to be on. src/main/booking/index.js."
  },
  "booking.errors.serverAnswer": {
    "message": "The server sent an answer Rukoo can't use. Try again later.",
    "description": "Booking page error: the list of bookings from the account server breaks the protocol's order rules. src/main/booking/index.js (fetchRows)."
  },
  "booking.defaults.typeTitle": {
    "message": "{minutes} minute meeting",
    "description": "Booking page: the title Rukoo gives a meeting type of one length, in the booking page's language, such as the first type's \"30 minute meeting\". minutes is the length. The event of a booking at another length gets this title with that length. src/main/booking/rules.js (defaultType, titleFor); src/renderer/booking-settings.js (meeting type editor)."
  },
  "booking.defaults.typeTitleAny": {
    "message": "Meeting",
    "description": "Booking page: the title Rukoo gives a meeting type once guests can pick from several lengths, replacing a title such as \"30 minute meeting\" it made before, in the booking page's language. src/renderer/booking-settings.js (meeting type editor)."
  },
  "booking.event.title": {
    "message": "{type}: {host} and {guest}",
    "description": "Title of the calendar event a booking becomes, in the booking page's language. type is the meeting type's title, host the page's display name, guest the guest's cleaned short name. src/main/booking/index.js (eventInput)."
  },
  "booking.event.titleNoGuest": {
    "message": "{type}: {host}",
    "description": "Title of the calendar event a booking becomes when the guest's name has nothing usable. type is the meeting type's title, host the page's display name. src/main/booking/index.js (eventInput)."
  },
  "booking.event.reschedule": {
    "message": "Pick another time: {url}",
    "description": "Line in a booked event's description, in the booking page's language; the guest reads it in their invitation. url is the long link that opens the booking page to move this booking; keep it at the end of the line. Same words as the button on the guest's manage page (server/src/booking-text.js manage.reschedule). src/main/booking/index.js (guestLines)."
  },
  "booking.event.cancel": {
    "message": "Cancel booking: {url}",
    "description": "Last line of a booked event's description, in the booking page's language; the guest reads it in their invitation. url is the long link to the guest's manage page, which has the Cancel button (opening it cancels nothing); keep it at the end of the line. Same words as that button (server/src/booking-text.js manage.cancel). src/main/booking/index.js (guestLines)."
  },
  "booking.event.video": {
    "message": "Join with {service}: {url}",
    "description": "First line of a booked event's description when its meeting type uses a video service, in the booking page's language; guests see it in their invitation. service is Zoom, Microsoft Teams or Jitsi Meet; url the meeting's link. src/main/video/index.js (withMeeting)."
  },
  "booking.decline.subject": {
    "message": "{type} on {when}",
    "description": "Subject of the optional email to a guest whose request the host declined, in the booking page's language. type is the meeting type, when the requested time. src/main/booking/index.js (declineMail)."
  },
  "booking.decline.body": {
    "message": "Sorry, I can't make {type} on {when}.",
    "description": "Body of the optional decline email when the host wrote no message, in the booking page's language, written as the host. src/main/booking/index.js (declineMail)."
  },
  "booking.decline.again": {
    "message": "You can pick another time here: {url}",
    "description": "Last line of the optional decline email, in the booking page's language. url is the booking page. src/main/booking/index.js (declineMail)."
  },
  "booking.notifications.newTitle": {
    "message": "New booking",
    "description": "Desktop notification or toast title when someone books time on the booking page. src/main/booking/index.js (tell)."
  },
  "booking.notifications.newBody": {
    "message": "{guest} booked {type} for {when}.",
    "description": "Desktop notification text for a new booking. guest is the guest's cleaned short name, type the meeting type, when the day and time. src/main/booking/index.js (tell)."
  },
  "booking.notifications.requestTitle": {
    "message": "Booking request",
    "description": "Desktop notification title for a booking that waits for the host's approval. src/main/booking/index.js (tell)."
  },
  "booking.notifications.requestBody": {
    "message": "{guest} asks for {type} on {when}. Accept or decline it in Rukoo.",
    "description": "Desktop notification text for a booking request. guest is the guest's cleaned short name, type the meeting type, when the day and time. src/main/booking/index.js (tell)."
  },
  "booking.notifications.rescheduledTitle": {
    "message": "Booking moved",
    "description": "Desktop notification title when a guest moves a booking. src/main/booking/index.js (tell)."
  },
  "booking.notifications.rescheduledBody": {
    "message": "{guest} moved {type} to {when}.",
    "description": "Desktop notification text when a guest moves a booking. guest is the guest's cleaned short name, type the meeting type, when the new day and time. src/main/booking/index.js (tell)."
  },
  "booking.notifications.cancelledTitle": {
    "message": "Booking cancelled",
    "description": "Desktop notification title when a guest cancels a booking. src/main/booking/index.js (tell)."
  },
  "booking.notifications.cancelledBody": {
    "message": "{guest} cancelled {type} on {when}.",
    "description": "Desktop notification text when a guest cancels. guest is the guest's cleaned short name, type the meeting type, when the day and time. src/main/booking/index.js (tell)."
  },
  "booking.notifications.withdrawnTitle": {
    "message": "Request withdrawn",
    "description": "Desktop notification title when a guest cancels a booking request the host hadn't answered yet. src/main/booking/index.js (tell)."
  },
  "booking.notifications.withdrawnBody": {
    "message": "{guest} withdrew the request for {type} on {when}.",
    "description": "Desktop notification text when a guest cancels a booking request. guest is the guest's cleaned short name, type the meeting type, when the requested day and time. src/main/booking/index.js (tell)."
  },
  "booking.notifications.someone": {
    "message": "A guest",
    "description": "Booking notifications and notices: stands in for a guest whose name has nothing usable. src/main/booking/index.js."
  },
  "booking.notices.actions.keep": {
    "message": "Keep",
    "description": "Booking notice button: keep a booking that overlaps another event. src/main/booking/index.js (notices)."
  },
  "booking.notices.actions.cancel": {
    "message": "Cancel booking",
    "description": "Booking notice button: cancel the booking the notice is about. src/main/booking/index.js (notices)."
  },
  "booking.notices.actions.reconnect": {
    "message": "Reconnect",
    "description": "Booking notice button: sign in to a calendar account again. src/main/booking/index.js (notices)."
  },
  "booking.notices.actions.takeover": {
    "message": "Use this computer",
    "description": "Booking notice button: make this computer the one that handles bookings. src/main/booking/index.js (notices)."
  },
  "booking.notices.actions.dismiss": {
    "message": "Dismiss",
    "description": "Booking notice button: hide the notice. src/main/booking/index.js (notices)."
  },
  "booking.notices.actions.open": {
    "message": "Open",
    "description": "Booking notice button: open the booked event or request. src/main/booking/index.js (notices)."
  },
  "booking.notices.actions.video": {
    "message": "Video meetings",
    "description": "Booking notice button: opens Settings > Video meetings to connect or renew a video service. src/main/booking/index.js (notices)."
  },
  "booking.notices.handleLostTitle": {
    "message": "Your booking page's address is gone",
    "description": "Booking notice title: another account now owns the page's address. src/main/booking/index.js (notices)."
  },
  "booking.notices.handleLostText": {
    "message": "Another account has {handle} now. Turn your booking page on again with a new address.",
    "description": "Booking notice text when the page's address was lost. handle is the old address. src/main/booking/index.js (notices)."
  },
  "booking.notices.otherDeviceTitle": {
    "message": "Bookings are handled by {device}",
    "description": "Booking notice title on a computer that isn't the booking device. device is that computer's name. src/main/booking/index.js (notices)."
  },
  "booking.notices.otherDeviceText": {
    "message": "That computer publishes your free times and adds bookings to your calendar.",
    "description": "Booking notice text on a computer that isn't the booking device. src/main/booking/index.js (notices)."
  },
  "booking.notices.otherDeviceVideoText": {
    "message": "That computer publishes your free times and adds bookings to your calendar. Before you use this computer instead, connect {services} here in Settings → Video meetings.",
    "description": "Booking notice text on a computer that isn't the booking device, when meeting types use video services that aren't connected on this computer. services lists them, such as 'Zoom and Microsoft Teams'. src/main/booking/index.js (notices)."
  },
  "booking.notices.otherDeviceUnknown": {
    "message": "another computer",
    "description": "Booking notices: stands in for the booking device's name when the server doesn't know it, inside a sentence. src/main/booking/index.js (notices)."
  },
  "booking.notices.keyTitle": {
    "message": "Guest details can't be opened",
    "description": "Booking notice title: the key guest details are sealed to is missing, different or damaged on this computer. src/main/booking/index.js (notices)."
  },
  "booking.notices.keyMissingText": {
    "message": "This computer doesn't have the key for your booking page yet. Sync your Rukoo account.",
    "description": "Booking notice text: the key pair hasn't reached this computer. src/main/booking/index.js (notices)."
  },
  "booking.notices.keyMismatchText": {
    "message": "Your booking page uses a key this computer doesn't have. Sync your Rukoo account, or turn the page off and on again.",
    "description": "Booking notice text: the published key isn't in this computer's keyring. src/main/booking/index.js (notices)."
  },
  "booking.notices.keyUnreadableText": {
    "message": "The key for your booking page on this computer is damaged, so new bookings can't be opened.",
    "description": "Booking notice text: the keyring itself doesn't open on this computer. src/main/booking/index.js (notices)."
  },
  "booking.notices.unopenedTitle": {
    "message": { "one": "{count} booking can't be opened", "other": "{count} bookings can't be opened" },
    "description": "Booking notice title: the guest details of upcoming bookings or requests couldn't be opened on this computer. count is how many. src/main/booking/index.js (notices)."
  },
  "booking.notices.unopenedText": {
    "message": { "one": "Rukoo couldn't open the guest details of {count} booking, so it isn't in your calendar. The details may be damaged, or locked with a key this computer doesn't have.", "other": "Rukoo couldn't open the guest details of {count} bookings, so they aren't in your calendar. The details may be damaged, or locked with a key this computer doesn't have." },
    "description": "Booking notice text when guest details couldn't be opened. count is how many bookings. src/main/booking/index.js (notices)."
  },
  "booking.notices.staleTitle": {
    "message": "Your booking page is out of date",
    "description": "Booking notice title: the free times weren't published for over 24 hours. src/main/booking/index.js (notices)."
  },
  "booking.notices.staleText": {
    "message": "Rukoo hasn't updated your free times for over 24 hours, so new bookings wait for your approval.",
    "description": "Booking notice text on the booking device when its last publish is over 24 hours old. src/main/booking/index.js (notices)."
  },
  "booking.notices.staleOtherText": {
    "message": "{device} hasn't updated your free times for over 24 hours, so new bookings wait for your approval. Start Rukoo there, or use this computer.",
    "description": "Booking notice text on another computer when the booking device hasn't published for over 24 hours. device is the booking device's name. src/main/booking/index.js (notices)."
  },
  "booking.notices.capTitle": {
    "message": "Today's booking limit reached",
    "description": "Booking notice title: the page took as many bookings today as its daily limit. src/main/booking/index.js (notices)."
  },
  "booking.notices.capText": {
    "message": { "one": "Your booking page took {count} booking today, its daily limit. It offers times again tomorrow.", "other": "Your booking page took {count} bookings today, its daily limit. It offers times again tomorrow." },
    "description": "Booking notice text when the daily limit is reached. count is the limit. src/main/booking/index.js (notices)."
  },
  "booking.notices.uncheckedTitle": {
    "message": "Couldn't check {email}",
    "description": "Booking notice title: a calendar account that counts toward availability couldn't be read for 12 hours. email is that account's address. src/main/booking/index.js (notices)."
  },
  "booking.notices.uncheckedText": {
    "message": "Rukoo couldn't read this calendar for 12 hours, so every booking now waits for your approval. Reconnect the account to fix this.",
    "description": "Booking notice text when a calendar account couldn't be read for 12 hours. src/main/booking/index.js (notices)."
  },
  "booking.notices.publishFailedTitle": {
    "message": "Couldn't update your booking page",
    "description": "Booking notice title: the last publish of free times failed. src/main/booking/index.js (notices)."
  },
  "booking.notices.publishFailedText": {
    "message": "Rukoo tries again in a few minutes. {error}",
    "description": "Booking notice text when publishing failed. error is the localised reason. src/main/booking/index.js (notices)."
  },
  "booking.notices.destinationTitle": {
    "message": "New bookings can't reach your calendar",
    "description": "Booking notice title on the booking device: the calendar bookings are written to isn't on this computer or can't be written. src/main/booking/index.js (notices)."
  },
  "booking.notices.destinationText": {
    "message": "The calendar for bookings, in {email}, isn't on this computer. New and moved bookings get no event and no invitation until you choose another calendar in Settings → Booking page.",
    "description": "Booking notice text when the destination calendar is missing on the booking device. email is the mail account the calendar belongs to. src/main/booking/index.js (notices)."
  },
  "booking.notices.videoMissingTitle": {
    "message": "No {service} links for new bookings",
    "description": "Booking notice title on the booking device: a meeting type uses a video service that can't make meetings on this computer. service is Zoom, Microsoft Teams or Jitsi Meet. src/main/booking/index.js (notices)."
  },
  "booking.notices.videoMissingText": {
    "message": "{service} isn't connected on this computer, so new bookings of {types} go in your calendar without a link. Connect it in Settings → Video meetings.",
    "description": "Booking notice text when a video service a meeting type uses isn't connected on the booking device. service is Zoom, Microsoft Teams or Jitsi Meet; types lists the meeting types' titles. src/main/booking/index.js (notices)."
  },
  "booking.notices.videoRenewText": {
    "message": "The {service} connection on this computer needs renewing, so new bookings of {types} go in your calendar without a link. Renew it in Settings → Video meetings.",
    "description": "Booking notice text when the sign-in to a video service a meeting type uses stopped working on the booking device. service is Zoom or Microsoft Teams; types lists the meeting types' titles. src/main/booking/index.js (notices)."
  },
  "booking.notices.videoBookingsTitle": {
    "message": "{service} meetings can't follow their bookings",
    "description": "Booking notice title on the booking device: upcoming bookings have Zoom or Teams meetings, but that service can't be used on this computer, so moves and cancellations can't change the meetings. service is Zoom or Microsoft Teams. src/main/booking/index.js (notices)."
  },
  "booking.notices.videoBookingsText": {
    "message": { "one": "{why} Until that's fixed, Rukoo can't move or delete the {service} meeting of {count} upcoming booking.", "other": "{why} Until that's fixed, Rukoo can't move or delete the {service} meetings of {count} upcoming bookings." },
    "description": "Booking notice text for the title above. why is one sentence with the reason (not connected here, or the connection needs renewing), service the video service, count how many upcoming bookings have such a meeting. src/main/booking/index.js (notices)."
  },
  "booking.notices.videoCreateTitle": {
    "message": "No {service} link for a booking",
    "description": "Booking notice title: Rukoo couldn't make the video meeting for a new booking. service is Zoom, Microsoft Teams or Jitsi Meet. src/main/booking/index.js (storedNotice)."
  },
  "booking.notices.videoCreateText": {
    "message": "Rukoo couldn't make a {service} meeting for {type} with {guest} on {when}, so the booking is in your calendar without a link. {why} Add a link to the event yourself, or cancel the booking.",
    "description": "Booking notice text when a new booking's video meeting couldn't be made. service is the video service, type the meeting type, guest the cleaned short name, when the day and time, why one sentence with the reason. src/main/booking/index.js (storedNotice)."
  },
  "booking.notices.videoUpdateTitle": {
    "message": "{service} meeting not moved",
    "description": "Booking notice title: a booking moved, but its video meeting couldn't be moved along. service is Zoom or Microsoft Teams. src/main/booking/index.js (storedNotice)."
  },
  "booking.notices.videoUpdateText": {
    "message": "{type} with {guest} moved to {when}, but Rukoo couldn't move its {service} meeting. {why} The link in the invitation still works; change the meeting's time in {service} if you need to.",
    "description": "Booking notice text when a moved booking's video meeting kept its old time. type is the meeting type, guest the cleaned short name, when the new day and time, service the video service, why one sentence with the reason. src/main/booking/index.js (storedNotice)."
  },
  "booking.notices.videoDeleteTitle": {
    "message": "{service} meeting not deleted",
    "description": "Booking notice title: a booking was cancelled, but its video meeting couldn't be deleted. service is Zoom or Microsoft Teams. src/main/booking/index.js (storedNotice)."
  },
  "booking.notices.videoDeleteText": {
    "message": "{type} with {guest} on {when} is cancelled, but Rukoo couldn't delete its {service} meeting. {why} You can delete it in {service} yourself.",
    "description": "Booking notice text when a cancelled booking's video meeting is still there. type is the meeting type, guest the cleaned short name, when the day and time, service the video service, why one sentence with the reason. src/main/booking/index.js (storedNotice)."
  },
  "booking.notices.videoWhyRenew": {
    "message": "The {service} connection on this computer needs renewing.",
    "description": "Reason sentence inside a video booking notice: the stored sign-in stopped working. service is Zoom or Microsoft Teams. src/main/booking/index.js (storedNotice)."
  },
  "booking.notices.videoWhyOff": {
    "message": "{service} isn't connected on this computer.",
    "description": "Reason sentence inside a video booking notice: the service isn't connected on the booking device. service is Zoom, Microsoft Teams or Jitsi Meet. src/main/booking/index.js (storedNotice)."
  },
  "booking.notices.videoWhyFailed": {
    "message": "{service} didn't answer as expected.",
    "description": "Reason sentence inside a video booking notice when the service gave no message. service is Zoom or Microsoft Teams. src/main/booking/index.js (storedNotice)."
  },
  "booking.notices.overlapTitle": {
    "message": "A booking overlaps another event",
    "description": "Booking notice title: a new or moved booking overlaps another busy event. src/main/booking/index.js (notices)."
  },
  "booking.notices.overlapText": {
    "message": "{type} with {guest} on {when} overlaps something in your calendar. Keep both, or cancel the booking.",
    "description": "Booking notice text for an overlap. type is the meeting type, guest the cleaned short name, when the day and time. src/main/booking/index.js (notices)."
  },
  "booking.notices.mailBudgetTitle": {
    "message": "No email sent to a guest",
    "description": "Booking notice title: the page reached its daily email limit, so a calendar change went out without mail. src/main/booking/index.js (notices)."
  },
  "booking.notices.mailBudgetText": {
    "message": "Your booking page reached today's email limit, so Rukoo changed {type} with {guest} on {when} in your calendar without emailing them.",
    "description": "Booking notice text when a change was made without mail. type is the meeting type, guest the cleaned short name, when the day and time. src/main/booking/index.js (notices)."
  },
  "booking.notices.guestCancelledTitle": {
    "message": "{guest} cancelled",
    "description": "Booking notice title when a guest cancelled. guest is the cleaned short name; type and when are given too. src/main/booking/index.js (notices)."
  },
  "booking.notices.guestCancelledText": {
    "message": "{type} on {when} is off. Rukoo removed it from your calendar.",
    "description": "Booking notice text when a guest cancelled. type is the meeting type, when the day and time; guest is given too. src/main/booking/index.js (notices)."
  },
  "booking.notices.guestDeclinedTitle": {
    "message": "{guest} declined",
    "description": "Booking notice title when the guest declined the calendar invitation. guest is the cleaned short name. src/main/booking/index.js (notices)."
  },
  "booking.notices.guestDeclinedText": {
    "message": "{guest} declined the invitation for {type} on {when}, so Rukoo cancelled the booking.",
    "description": "Booking notice text when the guest declined the invitation. guest is the cleaned short name, type the meeting type, when the day and time. src/main/booking/index.js (notices)."
  },
  "booking.notices.hostDeletedTitle": {
    "message": "Booking cancelled",
    "description": "Booking notice title: the host deleted a booked event, so the booking was cancelled. src/main/booking/index.js (notices)."
  },
  "booking.notices.hostDeletedText": {
    "message": "You deleted {type} with {guest} on {when} from your calendar, so Rukoo cancelled the booking.",
    "description": "Booking notice text when the host deleted a booked event. type is the meeting type, guest the cleaned short name, when the day and time. src/main/booking/index.js (notices)."
  },
  "reminders.untitled": {
    "message": "(No title)",
    "description": "Desktop reminder notification title for an event without a title. src/main/calendar/reminder-runner.js."
  },
  "reminders.now": {
    "message": "Starts now",
    "description": "Desktop reminder notification text for an event that starts now. src/main/calendar/reminder-runner.js."
  },
  "reminders.in": {
    "message": { "one": "Starts in {count} minute", "other": "Starts in {count} minutes" },
    "description": "Desktop reminder notification text for an event starting within the hour. count is minutes. src/main/calendar/reminder-runner.js."
  },
  "reminders.started": {
    "message": { "one": "Started {count} minute ago", "other": "Started {count} minutes ago" },
    "description": "Desktop reminder notification text for a timed event that started up to 5 minutes ago. count is minutes. src/main/calendar/reminder-runner.js."
  },
  "reminders.at": {
    "message": "{day} at {time}",
    "description": "Desktop reminder notification text for an event more than an hour away. day is Today, Tomorrow or a date; time the start time. src/main/calendar/reminder-runner.js."
  },
  "reminders.today": {
    "message": "Today",
    "description": "Desktop reminder notifications: the day word for today, inside reminders.at and reminders.allDay. src/main/calendar/reminder-runner.js."
  },
  "reminders.tomorrow": {
    "message": "Tomorrow",
    "description": "Desktop reminder notifications: the day word for tomorrow, inside reminders.at and reminders.allDay. src/main/calendar/reminder-runner.js."
  },
  "reminders.allDay": {
    "message": "{day}, all day",
    "description": "Desktop reminder notification text for an all-day event. day is Today, Tomorrow or a date. src/main/calendar/reminder-runner.js."
  },
  "reminders.join": {
    "message": "Click to join",
    "description": "Desktop reminder notification line for an event with a video meeting link; a click opens the event card with its Join button. src/main/calendar/reminder-runner.js."
  },
  "reminders.many": {
    "message": { "one": "{count} event coming up", "other": "{count} events coming up" },
    "description": "Desktop reminder notification title when several reminders show at once. count is the number of events. src/main/calendar/reminder-runner.js."
  },
  "reminders.more": {
    "message": { "one": "and {count} more", "other": "and {count} more" },
    "description": "Last line of a combined desktop reminder when more events are due than it lists. count is how many more. src/main/calendar/reminder-runner.js."
  },
  "settings.groups.sync": {
    "message": "Sync",
    "description": "Settings overview: group title above the Rukoo account row. src/renderer/settings.js (render)."
  },
  "settings.rukoo.title": {
    "message": "Rukoo account",
    "description": "Settings > Rukoo account: page title and the row on the Settings overview. src/renderer/settings.js."
  },
  "settings.rukoo.rowSignedOut": {
    "message": "Keep your categories the same on all your devices",
    "description": "Settings > Rukoo account: description of the Rukoo account row on the Settings overview while signed out. src/renderer/settings.js."
  },
  "settings.rukoo.rowSignedIn": {
    "message": "Signed in as {email}",
    "description": "Settings > Rukoo account: description of the Rukoo account row while signed in. {email}: the account email address. src/renderer/settings.js."
  },
  "settings.rukoo.intro": {
    "message": "A Rukoo account keeps Rukoo's own data the same on your phone and computers: your inbox categories, the category each email got and your corrections. Rukoo encrypts it on this device before it leaves, and only your devices have the key. Your mail and your mail passwords never go to the account.",
    "description": "Settings > Rukoo account: explanation at the top of the page while signed out. src/renderer/settings.js."
  },
  "settings.rukoo.server": {
    "message": "Server",
    "description": "Settings > Rukoo account: row title for the account server the app uses. src/renderer/settings.js."
  },
  "settings.rukoo.serverNone": {
    "message": "No server set",
    "description": "Settings > Rukoo account: value of the Server row when no server address is configured. src/renderer/settings.js."
  },
  "settings.rukoo.serverRunBy": {
    "message": "Run by {operator}",
    "description": "Settings > Rukoo account: Server row description. {operator}: the name of whoever runs the server. src/renderer/settings.js."
  },
  "settings.rukoo.serverUnreachable": {
    "message": "Can't reach this server right now",
    "description": "Settings > Rukoo account: Server row description when the server info could not be fetched. src/renderer/settings.js."
  },
  "settings.rukoo.serverPrompt": {
    "message": "Account server address",
    "description": "Settings > Rukoo account: title of the dialog to enter another account server address. src/renderer/settings.js."
  },
  "settings.rukoo.serverSaved": {
    "message": "Account server changed",
    "description": "Settings > Rukoo account: toast after changing the account server. src/renderer/settings.js."
  },
  "settings.rukoo.where.eu": {
    "message": "Data stored in the EU",
    "description": "Settings > Rukoo account: where the server keeps account data: European Union jurisdiction. src/renderer/settings.js."
  },
  "settings.rukoo.where.us": {
    "message": "Data stored in the United States",
    "description": "Settings > Rukoo account: where the server keeps account data: US jurisdiction. src/renderer/settings.js."
  },
  "settings.rukoo.where.fedramp": {
    "message": "Data stored in FedRAMP data centers in the United States",
    "description": "Settings > Rukoo account: where the server keeps account data: FedRAMP jurisdiction. src/renderer/settings.js."
  },
  "settings.rukoo.where.any": {
    "message": "Data stored wherever Cloudflare places it",
    "description": "Settings > Rukoo account: where the server keeps account data when no jurisdiction is set. src/renderer/settings.js."
  },
  "settings.rukoo.signUp": {
    "message": "Create an account",
    "description": "Settings > Rukoo account: row and page title to sign up. src/renderer/settings.js."
  },
  "settings.rukoo.signIn": {
    "message": "Sign in",
    "description": "Settings > Rukoo account: row, page title and button to sign in. src/renderer/settings.js."
  },
  "settings.rukoo.forgot": {
    "message": "Forgot your password?",
    "description": "Settings > Rukoo account: row that opens the recovery-key form. src/renderer/settings.js."
  },
  "settings.rukoo.privacy": {
    "message": "What's stored and shared",
    "description": "Settings > Rukoo account: row and page title of the data and privacy page. src/renderer/settings.js."
  },
  "settings.rukoo.privacyHelp": {
    "message": "Where your account's data lives and who can see what",
    "description": "Settings > Rukoo account: description of the data and privacy row. src/renderer/settings.js."
  },
  "settings.rukoo.passwordHelp": {
    "message": "At least 12 characters. Nobody can reset it for you, so choose one you'll remember or keep it in a password manager.",
    "description": "Settings > Rukoo account: help under the password field on the sign-up and recovery forms. src/renderer/settings.js."
  },
  "settings.rukoo.repeat": {
    "message": "Repeat password",
    "description": "Settings > Rukoo account: label of the second password field on the sign-up and recovery forms. src/renderer/settings.js."
  },
  "settings.rukoo.mismatch": {
    "message": "The passwords don't match.",
    "description": "Settings > Rukoo account: form error when both password fields differ. src/renderer/settings.js."
  },
  "settings.rukoo.invite": {
    "message": "Invite code",
    "description": "Settings > Rukoo account: label of the invite code field on the sign-up form, shown when the server is invite-only. src/renderer/settings.js."
  },
  "settings.rukoo.understand": {
    "message": "I understand that if I forget my password, only my recovery key gets me back in.",
    "description": "Settings > Rukoo account: checkbox on the sign-up form. src/renderer/settings.js."
  },
  "settings.rukoo.needUnderstand": {
    "message": "Tick the box to confirm.",
    "description": "Settings > Rukoo account: form error when the sign-up checkbox is not ticked. src/renderer/settings.js."
  },
  "settings.rukoo.create": {
    "message": "Create account",
    "description": "Settings > Rukoo account: submit button of the sign-up form. src/renderer/settings.js."
  },
  "settings.rukoo.working": {
    "message": "Working…",
    "description": "Settings > Rukoo account: submit button text while the password is hashed and the server answers. src/renderer/settings.js."
  },
  "settings.rukoo.recoveryKeyField": {
    "message": "Recovery key",
    "description": "Settings > Rukoo account: label of the recovery key field on the forgot-password form. src/renderer/settings.js."
  },
  "settings.rukoo.newPassword": {
    "message": "New password",
    "description": "Settings > Rukoo account: label of the new password field. src/renderer/settings.js."
  },
  "settings.rukoo.recoverHelp": {
    "message": "Enter the recovery key you saved when you made the account, and choose a new password. Your other devices sign out and need the new password.",
    "description": "Settings > Rukoo account: explanation on the forgot-password form. src/renderer/settings.js."
  },
  "settings.rukoo.recoverSubmit": {
    "message": "Set new password",
    "description": "Settings > Rukoo account: submit button of the forgot-password form. src/renderer/settings.js."
  },
  "settings.rukoo.closed": {
    "message": "This server doesn't take new accounts. Sign in with an existing account, or choose another server.",
    "description": "Settings > Rukoo account: note on the sign-up page when the server has sign-ups closed. src/renderer/settings.js."
  },
  "settings.rukoo.keyTitle": {
    "message": "Save your recovery key",
    "description": "Settings > Rukoo account: title of the dialog that shows the recovery key once. src/renderer/settings.js."
  },
  "settings.rukoo.keyHelp": {
    "message": "If you forget your password, this key is the only way back into your account. Rukoo shows it only now. Keep it somewhere safe, such as a password manager.",
    "description": "Settings > Rukoo account: explanation in the recovery key dialog. src/renderer/settings.js."
  },
  "settings.rukoo.copy": {
    "message": "Copy",
    "description": "Settings > Rukoo account: button in the recovery key dialog that copies the key. src/renderer/settings.js."
  },
  "settings.rukoo.copied": {
    "message": "Recovery key copied",
    "description": "Settings > Rukoo account: toast after copying the recovery key. src/renderer/settings.js."
  },
  "settings.rukoo.keySaved": {
    "message": "I saved it",
    "description": "Settings > Rukoo account: button that closes the recovery key dialog. src/renderer/settings.js."
  },
  "settings.rukoo.welcome": {
    "message": "Account created. This computer now syncs.",
    "description": "Settings > Rukoo account: toast after signing up. src/renderer/settings.js."
  },
  "settings.rukoo.signedIn": {
    "message": "Signed in. This computer now syncs.",
    "description": "Settings > Rukoo account: toast after signing in or recovering. src/renderer/settings.js."
  },
  "settings.rukoo.account": {
    "message": "Account",
    "description": "Settings > Rukoo account: group title above the signed-in account row. src/renderer/settings.js."
  },
  "settings.rukoo.neverSynced": {
    "message": "Not synced yet",
    "description": "Settings > Rukoo account: sync status before the first sync. src/renderer/settings.js."
  },
  "settings.rukoo.syncing": {
    "message": "Syncing…",
    "description": "Settings > Rukoo account: sync status while syncing. src/renderer/settings.js."
  },
  "settings.rukoo.syncNow": {
    "message": "Sync now",
    "description": "Settings > Rukoo account: row that syncs right away. src/renderer/settings.js."
  },
  "settings.rukoo.synced": {
    "message": "Synced",
    "description": "Settings > Rukoo account: toast after a manual sync. src/renderer/settings.js."
  },
  "settings.rukoo.pending": {
    "message": {"one": "{count} change waiting to sync", "other": "{count} changes waiting to sync"},
    "description": "Settings > Rukoo account: sync status when local changes have not reached the server yet. {count}: number of changes. src/renderer/settings.js."
  },
  "settings.rukoo.devices": {
    "message": "Devices",
    "description": "Settings > Rukoo account: group title above the list of signed-in devices. src/renderer/settings.js."
  },
  "settings.rukoo.thisDevice": {
    "message": "This device",
    "description": "Settings > Rukoo account: description of the device row for this computer. src/renderer/settings.js."
  },
  "settings.rukoo.lastSeen": {
    "message": "Last seen on {date}",
    "description": "Settings > Rukoo account: description of another device row. {date}: when it last connected. src/renderer/settings.js."
  },
  "settings.rukoo.devicesUnavailable": {
    "message": "Can't load your devices right now.",
    "description": "Settings > Rukoo account: shown instead of the device list when the server could not be reached. src/renderer/settings.js."
  },
  "settings.rukoo.removeDevice": {
    "message": "Sign out {name}?",
    "description": "Settings > Rukoo account: title of the dialog to sign another device out. {name}: device name. src/renderer/settings.js."
  },
  "settings.rukoo.removeDeviceHelp": {
    "message": "{name} stops syncing with this account. It keeps its own copy.",
    "description": "Settings > Rukoo account: text of the dialog to sign another device out. src/renderer/settings.js."
  },
  "settings.rukoo.removeDeviceAction": {
    "message": "Sign out",
    "description": "Settings > Rukoo account: confirm button of the dialog to sign another device out. src/renderer/settings.js."
  },
  "settings.rukoo.deviceRemoved": {
    "message": "Device signed out",
    "description": "Settings > Rukoo account: toast after signing another device out. src/renderer/settings.js."
  },
  "settings.rukoo.platform.win32": {
    "message": "Windows",
    "description": "Settings > Rukoo account: platform name in a device row. src/renderer/settings.js."
  },
  "settings.rukoo.platform.darwin": {
    "message": "macOS",
    "description": "Settings > Rukoo account: platform name in a device row. src/renderer/settings.js."
  },
  "settings.rukoo.platform.linux": {
    "message": "Linux",
    "description": "Settings > Rukoo account: platform name in a device row. src/renderer/settings.js."
  },
  "settings.rukoo.platform.android": {
    "message": "Android",
    "description": "Settings > Rukoo account: platform name in a device row. src/renderer/settings.js."
  },
  "settings.rukoo.security": {
    "message": "Security",
    "description": "Settings > Rukoo account: group title above password and recovery key rows. src/renderer/settings.js."
  },
  "settings.rukoo.changePassword": {
    "message": "Change password",
    "description": "Settings > Rukoo account: row and dialog title to change the password. src/renderer/settings.js."
  },
  "settings.rukoo.changePasswordHelp": {
    "message": "Your other devices sign out and need the new password.",
    "description": "Settings > Rukoo account: description of the change password row. src/renderer/settings.js."
  },
  "settings.rukoo.currentPassword": {
    "message": "Current password",
    "description": "Settings > Rukoo account: label of the current password field. src/renderer/settings.js."
  },
  "settings.rukoo.passwordChanged": {
    "message": "Password changed",
    "description": "Settings > Rukoo account: toast after changing the password. src/renderer/settings.js."
  },
  "settings.rukoo.newKey": {
    "message": "New recovery key",
    "description": "Settings > Rukoo account: row that makes a new recovery key. src/renderer/settings.js."
  },
  "settings.rukoo.newKeyHelp": {
    "message": "Your current recovery key stops working.",
    "description": "Settings > Rukoo account: description of the new recovery key row. src/renderer/settings.js."
  },
  "settings.rukoo.enterPassword": {
    "message": "Enter your password",
    "description": "Settings > Rukoo account: title of the dialog that asks for the password before a sensitive action. src/renderer/settings.js."
  },
  "settings.rukoo.continue": {
    "message": "Continue",
    "description": "Settings > Rukoo account: confirm button of the password dialog. src/renderer/settings.js."
  },
  "settings.rukoo.yourData": {
    "message": "Your data",
    "description": "Settings > Rukoo account: group title above the privacy and export rows. src/renderer/settings.js."
  },
  "settings.rukoo.export": {
    "message": "Export my data",
    "description": "Settings > Rukoo account: row and save dialog title to export the account data. src/renderer/settings.js."
  },
  "settings.rukoo.exportHelp": {
    "message": "Save everything in your account as a readable JSON file",
    "description": "Settings > Rukoo account: description of the export row. src/renderer/settings.js."
  },
  "settings.rukoo.exported": {
    "message": "Data exported",
    "description": "Settings > Rukoo account: toast after saving the export. src/renderer/settings.js."
  },
  "settings.rukoo.signOut": {
    "message": "Sign out",
    "description": "Settings > Rukoo account: row that signs this computer out. src/renderer/settings.js."
  },
  "settings.rukoo.signOutHelp": {
    "message": "This computer stops syncing and keeps its own copy of your categories.",
    "description": "Settings > Rukoo account: description of the sign out row. src/renderer/settings.js."
  },
  "settings.rukoo.signedOut": {
    "message": "Signed out",
    "description": "Settings > Rukoo account: toast after signing out. src/renderer/settings.js."
  },
  "settings.rukoo.delete": {
    "message": "Delete account",
    "description": "Settings > Rukoo account: row and dialog title to delete the account. src/renderer/settings.js."
  },
  "settings.rukoo.deleteHelp": {
    "message": "Deletes your account and everything in it from the server. Your devices keep their own copies and stop syncing. This cannot be undone.",
    "description": "Settings > Rukoo account: description of the delete row and text of its confirmation. src/renderer/settings.js."
  },
  "settings.rukoo.deleted": {
    "message": "Account deleted",
    "description": "Settings > Rukoo account: toast after deleting the account. src/renderer/settings.js."
  },
  "settings.rukoo.stored": {
    "message": "Stored in your account",
    "description": "Settings > Rukoo account: privacy page: group title above the counts of stored records while signed in. src/renderer/settings.js."
  },
  "settings.rukoo.storedLocal": {
    "message": "Stored on this computer",
    "description": "Settings > Rukoo account: privacy page: group title above the counts while signed out. src/renderer/settings.js."
  },
  "settings.rukoo.storedHelp": {
    "message": "Encrypted on your device before it leaves. Only your devices have the key.",
    "description": "Settings > Rukoo account: privacy page: note under the stored counts. src/renderer/settings.js."
  },
  "settings.rukoo.count.category": {
    "message": {"one": "{count} category", "other": "{count} categories"},
    "description": "Settings > Rukoo account: privacy page: number of inbox categories stored. src/renderer/settings.js."
  },
  "settings.rukoo.count.classification": {
    "message": {"one": "{count} sorted email", "other": "{count} sorted emails"},
    "description": "Settings > Rukoo account: privacy page: number of emails with a stored category. src/renderer/settings.js."
  },
  "settings.rukoo.count.correction": {
    "message": {"one": "{count} correction", "other": "{count} corrections"},
    "description": "Settings > Rukoo account: privacy page: number of corrections the user made. src/renderer/settings.js."
  },
  "settings.rukoo.serverUsage": {
    "message": "{size} on the server, encrypted",
    "description": "Settings > Rukoo account: privacy page: how much the account uses on the server. {size}: for example 12 KB. src/renderer/settings.js."
  },
  "settings.rukoo.canSee": {
    "message": "What the server can see",
    "description": "Settings > Rukoo account: privacy page: group title. src/renderer/settings.js."
  },
  "settings.rukoo.see.email": {
    "message": "Your email address",
    "description": "Settings > Rukoo account: privacy page: item the server can see. src/renderer/settings.js."
  },
  "settings.rukoo.see.devices": {
    "message": "Your devices' names and when they last connected",
    "description": "Settings > Rukoo account: privacy page: item the server can see. src/renderer/settings.js."
  },
  "settings.rukoo.see.records": {
    "message": "How many records you have, how large they are and when they change",
    "description": "Settings > Rukoo account: privacy page: item the server can see. src/renderer/settings.js."
  },
  "settings.rukoo.see.ip": {
    "message": "Your IP address, in the hosting provider's logs",
    "description": "Settings > Rukoo account: privacy page: item the server can see. src/renderer/settings.js."
  },
  "settings.rukoo.cantSee": {
    "message": "What it can't see",
    "description": "Settings > Rukoo account: privacy page: group title. src/renderer/settings.js."
  },
  "settings.rukoo.cantSeeHelp": {
    "message": "Your categories, your corrections, which email went where, and anything in your mail.",
    "description": "Settings > Rukoo account: privacy page: what stays unreadable to the server. src/renderer/settings.js."
  },
  "settings.rukoo.services": {
    "message": "Who handles your data",
    "description": "Settings > Rukoo account: privacy page: group title above the list of parties. src/renderer/settings.js."
  },
  "settings.rukoo.operatorHelp": {
    "message": "Runs the server. Gets your email address, your device names and your encrypted records.",
    "description": "Settings > Rukoo account: privacy page: what the server operator handles. src/renderer/settings.js."
  },
  "settings.rukoo.hostHelp": {
    "message": "Hosts the server, from {country}. {where}. Handles what the server sees, plus IP addresses in its logs.",
    "description": "Settings > Rukoo account: privacy page: what the hosting company handles. {country}: country name. {where}: a sentence like \"Data stored in the EU\". src/renderer/settings.js."
  },
  "settings.rukoo.usLaw": {
    "message": "As a US company it falls under US law, also for data stored in the EU.",
    "description": "Settings > Rukoo account: privacy page: note when the hosting company is based in the United States. src/renderer/settings.js."
  },
  "settings.rukoo.country.US": {
    "message": "the United States",
    "description": "Settings > Rukoo account: privacy page: country name used in hostHelp. src/renderer/settings.js."
  },
  "settings.rukoo.mailProviders": {
    "message": "Your mail providers",
    "description": "Settings > Rukoo account: privacy page: party title. src/renderer/settings.js."
  },
  "settings.rukoo.mailProvidersHelp": {
    "message": "Keep your mail as always. Nothing from your Rukoo account goes to them.",
    "description": "Settings > Rukoo account: privacy page: what mail providers handle. src/renderer/settings.js."
  },
  "settings.rukoo.nobodyElse": {
    "message": "Nobody else",
    "description": "Settings > Rukoo account: privacy page: party title. src/renderer/settings.js."
  },
  "settings.rukoo.nobodyElseHelp": {
    "message": "Rukoo uses no analytics, advertising or tracking services.",
    "description": "Settings > Rukoo account: privacy page: there are no other parties. src/renderer/settings.js."
  },
  "settings.rukoo.policy": {
    "message": "Privacy policy",
    "description": "Settings > Rukoo account: privacy page: row that opens the server operator's privacy policy. src/renderer/settings.js."
  },
  "settings.rukoo.policyHelp": {
    "message": "{operator}'s privacy policy",
    "description": "Settings > Rukoo account: privacy page: description of the privacy policy row. {operator}: who runs the server. src/renderer/settings.js."
  },
  "settings.rukoo.protocol": {
    "message": "How the encryption works",
    "description": "Settings > Rukoo account: privacy page: row that opens Rukoo's account protocol on GitHub. src/renderer/settings.js."
  },
  "settings.rukoo.protocolHelp": {
    "message": "Rukoo's account protocol on GitHub",
    "description": "Settings > Rukoo account: privacy page: description of the protocol row. src/renderer/settings.js."
  },
  "settings.rukoo.selfHost": {
    "message": "Run your own server",
    "description": "Settings > Rukoo account: privacy page: row that opens the self-hosting guide. src/renderer/settings.js."
  },
  "settings.rukoo.selfHostHelp": {
    "message": "Rukoo is open source. The guide shows how to host your own account server.",
    "description": "Settings > Rukoo account: privacy page: description of the self-hosting row. src/renderer/settings.js."
  },
  "categories.presets.needs-you.name": {
    "message": "Needs you",
    "description": "Inbox categories: the name of Rukoo's own \"Needs you\" category until the person renames it. A tab above the inbox and a row in Settings. src/main/categories/index.js (list)."
  },
  "categories.presets.needs-you.description": {
    "message": "Mail from people that asks you to do something: reply, decide, approve, sign, pay or show up.",
    "description": "Inbox categories: what belongs in the \"Needs you\" category. Shown in Settings and read by Jev, the model that sorts mail, so it must describe the mail clearly. src/main/categories/index.js (list)."
  },
  "categories.presets.to-read.name": {
    "message": "To read",
    "description": "Inbox categories: the name of Rukoo's own \"To read\" category until the person renames it. A tab above the inbox and a row in Settings. src/main/categories/index.js (list)."
  },
  "categories.presets.to-read.description": {
    "message": "Mail from people or organisations worth reading that needs nothing from you: news from colleagues, threads you are copied on, personal updates.",
    "description": "Inbox categories: what belongs in the \"To read\" category. Shown in Settings and read by Jev, the model that sorts mail, so it must describe the mail clearly. src/main/categories/index.js (list)."
  },
  "categories.presets.money.name": {
    "message": "Money",
    "description": "Inbox categories: the name of Rukoo's own \"Money\" category until the person renames it. A tab above the inbox and a row in Settings. src/main/categories/index.js (list)."
  },
  "categories.presets.money.description": {
    "message": "Invoices, payment requests, receipts, order confirmations, bank and tax mail, subscriptions and refunds.",
    "description": "Inbox categories: what belongs in the \"Money\" category. Shown in Settings and read by Jev, the model that sorts mail, so it must describe the mail clearly. src/main/categories/index.js (list)."
  },
  "categories.presets.updates.name": {
    "message": "Updates",
    "description": "Inbox categories: the name of Rukoo's own \"Updates\" category until the person renames it. A tab above the inbox and a row in Settings. src/main/categories/index.js (list)."
  },
  "categories.presets.updates.description": {
    "message": "Automatic notifications: deliveries, account and security alerts, comments and mentions in apps, social networks and forums.",
    "description": "Inbox categories: what belongs in the \"Updates\" category. Shown in Settings and read by Jev, the model that sorts mail, so it must describe the mail clearly. src/main/categories/index.js (list)."
  },
  "categories.presets.promotions.name": {
    "message": "Promotions",
    "description": "Inbox categories: the name of Rukoo's own \"Promotions\" category until the person renames it. A tab above the inbox and a row in Settings. src/main/categories/index.js (list)."
  },
  "categories.presets.promotions.description": {
    "message": "Newsletters, marketing, offers and discounts, and events or webinars someone is selling.",
    "description": "Inbox categories: what belongs in the \"Promotions\" category. Shown in Settings and read by Jev, the model that sorts mail, so it must describe the mail clearly. src/main/categories/index.js (list)."
  },
  "categories.errors.tooMany": {
    "message": "You can have up to {count} categories.",
    "description": "Inbox categories: error when adding a category past the limit. {count}: the limit. src/main/categories/index.js (saveCategory)."
  },
  "categories.errors.needName": {
    "message": "Give the category a name.",
    "description": "Inbox categories: error when saving a new category without a name. src/main/categories/index.js (saveCategory)."
  },
  "categories.errors.last": {
    "message": "Keep at least one category. To stop sorting, turn categories off.",
    "description": "Inbox categories: error when deleting the last category. src/main/categories/index.js (removeCategory)."
  },
  "mailbox.categories.label": {
    "message": "Inbox categories",
    "description": "Inbox categories in the main window: accessible name of the row of category tabs above the inbox. src/renderer/app.js."
  },
  "mailbox.categories.all": {
    "message": "All",
    "description": "Inbox categories in the main window: first category tab: the whole inbox. src/renderer/app.js."
  },
  "mailbox.categories.move": {
    "message": "Move to category",
    "description": "Inbox categories in the main window: message menu item that moves the email to another inbox category. src/renderer/app.js."
  },
  "mailbox.categories.pick": {
    "message": "Move to category",
    "description": "Inbox categories in the main window: title of the dialog that lists the categories to move emails to. src/renderer/app.js."
  },
  "mailbox.categories.moved": {
    "message": "Moved to {name}",
    "description": "Inbox categories in the main window: toast after moving emails to a category. {name}: the category. src/renderer/app.js."
  },
  "mailbox.categories.always": {
    "message": "Also future mail",
    "description": "Inbox categories in the main window: button in the toast after a move that also moves every later email from the same sender. src/renderer/app.js."
  },
  "mailbox.categories.alwaysDone": {
    "message": "Mail from {sender} goes to {name} from now on",
    "description": "Inbox categories in the main window: toast after choosing Also future mail for one sender. {sender}: the sender; {name}: the category. src/renderer/app.js."
  },
  "mailbox.categories.alwaysDoneMany": {
    "message": "Mail from these {count} senders goes to {name} from now on",
    "description": "Inbox categories in the main window: toast after choosing Also future mail for emails from several senders. {count}: number of senders; {name}: the category. src/renderer/app.js."
  },
  "mailbox.categories.empty": {
    "message": "Nothing in {name}",
    "description": "Inbox categories in the main window: empty inbox list when a category tab has no mail. {name}: the category. src/renderer/app.js."
  },
  "mailbox.categories.unreadCount": {
    "message": {"one": "{count} unread", "other": "{count} unread"},
    "description": "Inbox categories in the main window: accessible name of the unread count on a category tab. src/renderer/app.js."
  },
  "mailbox.categories.hidden": {
    "message": "Not shown in Inbox",
    "description": "Inbox categories: accessible name and tooltip of the crossed-out eye beside a category whose Show in Inbox switch is off, in the sidebar and in Settings > Inbox categories. src/renderer/app.js; src/renderer/categories-settings.js."
  },
  "mailbox.categories.hiddenTitle": {
    "message": "{name} isn't shown in Inbox",
    "description": "Inbox categories in the main window: tooltip of a category under Inbox in the sidebar whose Show in Inbox switch is off. {name}: the category. src/renderer/app.js."
  },
  "settings.general.categories": {
    "message": "Inbox categories",
    "description": "Settings > General: row that opens the inbox categories page. src/renderer/settings.js (render)."
  },
  "settings.general.categoriesOn": {
    "message": "On",
    "description": "Settings > General: value of the Inbox categories row when categories are on. src/renderer/settings.js (render)."
  },
  "settings.general.categoriesOff": {
    "message": "Off",
    "description": "Settings > General: value of the Inbox categories row when categories are off. src/renderer/settings.js (render)."
  },
  "settings.categories.title": {
    "message": "Inbox categories",
    "description": "Settings > Inbox categories: page title. src/renderer/categories-settings.js."
  },
  "settings.categories.on": {
    "message": "Sort the inbox into categories",
    "description": "Settings > Inbox categories: switch that turns categories on. src/renderer/categories-settings.js."
  },
  "settings.categories.onHelp": {
    "message": "The inbox gets tabs, like Needs you and Promotions. Nothing moves or changes on your mail server, and other mail apps see your inbox as before.",
    "description": "Settings > Inbox categories: help under the switch. src/renderer/categories-settings.js."
  },
  "settings.categories.list": {
    "message": "Categories",
    "description": "Settings > Inbox categories: group title above the list of categories. src/renderer/categories-settings.js."
  },
  "settings.categories.listHelp": {
    "message": "Each email goes to one category. When several fit, the one higher in the list wins.",
    "description": "Settings > Inbox categories: note under the list of categories. src/renderer/categories-settings.js."
  },
  "settings.categories.add": {
    "message": "Add a category",
    "description": "Settings > Inbox categories: row that adds a category. src/renderer/categories-settings.js."
  },
  "settings.categories.up": {
    "message": "Move up",
    "description": "Settings > Inbox categories: button that moves a category up the list. src/renderer/categories-settings.js."
  },
  "settings.categories.down": {
    "message": "Move down",
    "description": "Settings > Inbox categories: button that moves a category down the list. src/renderer/categories-settings.js."
  },
  "settings.categories.unsure": {
    "message": "When Rukoo isn't sure",
    "description": "Settings > Inbox categories: row: the category that gets mail Rukoo can't place. src/renderer/categories-settings.js."
  },
  "settings.categories.editTitle": {
    "message": "Edit category",
    "description": "Settings > Inbox categories: title of the dialog that edits a category. src/renderer/categories-settings.js."
  },
  "settings.categories.newTitle": {
    "message": "New category",
    "description": "Settings > Inbox categories: title of the dialog that adds a category. src/renderer/categories-settings.js."
  },
  "settings.categories.name": {
    "message": "Name",
    "description": "Settings > Inbox categories: label of the name field in the category dialog. src/renderer/categories-settings.js."
  },
  "settings.categories.description": {
    "message": "What belongs here",
    "description": "Settings > Inbox categories: label of the description field in the category dialog. src/renderer/categories-settings.js."
  },
  "settings.categories.descriptionHelp": {
    "message": "Rukoo, and Jev when it is on, read this to decide which mail goes here. Name the kinds of mail and senders.",
    "description": "Settings > Inbox categories: help under the description field. src/renderer/categories-settings.js."
  },
  "settings.categories.color": {
    "message": "Color",
    "description": "Settings > Inbox categories: label of the color choice in the category dialog. src/renderer/categories-settings.js."
  },
  "settings.categories.inbox": {
    "message": "Show in Inbox",
    "description": "Settings > Inbox categories: switch in the category dialog; off keeps the category's mail out of Inbox itself. Inbox is the label of mailbox.folders.inbox. src/renderer/categories-settings.js."
  },
  "settings.categories.inboxHelp": {
    "message": "When off, this mail stays out of Inbox and its unread count. You find it under the category in the sidebar, and search still finds it.",
    "description": "Settings > Inbox categories: help under the Show in Inbox switch in the category dialog. src/renderer/categories-settings.js."
  },
  "settings.categories.notify": {
    "message": "Notifications",
    "description": "Settings > Inbox categories: label of the switch in the category dialog that turns desktop notifications for the category's new mail on or off. src/renderer/categories-settings.js."
  },
  "settings.categories.notifyHelp": {
    "message": "Show a desktop notification for new mail in this category. This choice stays on this computer.",
    "description": "Settings > Inbox categories: help under the notifications switch in the category dialog. The choice isn't synced, so a phone can differ. src/renderer/categories-settings.js."
  },
  "settings.categories.notifyBlocked": {
    "message": "Notifications are off for all new mail, so this category stays quiet too. Turn on Notifications in Settings first.",
    "description": "Settings > Inbox categories: help under the disabled notifications switch in the category dialog while notifications are off for all mail (settings.general.notifications, whose label is the Notifications named here). src/renderer/categories-settings.js."
  },
  "settings.categories.notifyOff": {
    "message": "Notifications off",
    "description": "Settings > Inbox categories: tooltip and accessible name of the bell icon on a category row whose notifications are off. src/renderer/categories-settings.js."
  },
  "settings.categories.restore": {
    "message": "Use Rukoo's text",
    "description": "Settings > Inbox categories: button in the dialog that puts back the original name and description of one of Rukoo's own categories. src/renderer/categories-settings.js."
  },
  "settings.categories.deleteTitle": {
    "message": "Delete {name}?",
    "description": "Settings > Inbox categories: title of the confirmation before deleting a category. {name}: the category. src/renderer/categories-settings.js."
  },
  "settings.categories.deleteHelp": {
    "message": "Its mail goes to the other categories. Sender rules for it stop working.",
    "description": "Settings > Inbox categories: text of the confirmation before deleting a category. src/renderer/categories-settings.js."
  },
  "settings.categories.sorting": {
    "message": "Sorting",
    "description": "Settings > Inbox categories: group title above the sorting options. src/renderer/categories-settings.js."
  },
  "settings.categories.rules": {
    "message": "Rukoo sorts by the sender and the mail's headers, and by Gmail's own tabs for Gmail accounts. You can move any email, and choose to do the same for its sender from then on.",
    "description": "Settings > Inbox categories: explanation of how sorting works without Jev. src/renderer/categories-settings.js."
  },
  "settings.categories.jev": {
    "message": "Sort with Jev",
    "description": "Settings > Inbox categories: switch that turns on sorting with Jev, TypeSafe's decision model. src/renderer/categories-settings.js."
  },
  "settings.categories.jevHelp": {
    "message": "Jev, a decision model by TypeSafe, reads each new email and picks a category from your descriptions. Needs your own TypeSafe API key.",
    "description": "Settings > Inbox categories: help under the Jev switch. src/renderer/categories-settings.js."
  },
  "settings.categories.jevShares": {
    "message": "With Jev on, this computer sends the sender, the subject and the first 1,000 characters of each new inbox email, with your category names and descriptions, to TypeSafe, Inc. in the United States. TypeSafe's own terms apply to what it keeps. Nothing else leaves this computer.",
    "description": "Settings > Inbox categories: what turning Jev on shares, shown under the Jev rows. src/renderer/categories-settings.js."
  },
  "settings.categories.key": {
    "message": "TypeSafe API key",
    "description": "Settings > Inbox categories: row that sets the TypeSafe API key. src/renderer/categories-settings.js."
  },
  "settings.categories.keySet": {
    "message": "Set on this computer",
    "description": "Settings > Inbox categories: value of the API key row when a key is stored. src/renderer/categories-settings.js."
  },
  "settings.categories.keyMissing": {
    "message": "Not set",
    "description": "Settings > Inbox categories: value of the API key row without a key. src/renderer/categories-settings.js."
  },
  "settings.categories.keyPrompt": {
    "message": "TypeSafe API key",
    "description": "Settings > Inbox categories: title of the dialog to enter the API key. src/renderer/categories-settings.js."
  },
  "settings.categories.keyRemove": {
    "message": "Remove key",
    "description": "Settings > Inbox categories: button in the API key dialog that removes the stored key. src/renderer/categories-settings.js."
  },
  "settings.categories.needKey": {
    "message": "Add your TypeSafe API key first.",
    "description": "Settings > Inbox categories: toast when turning Jev on without a key. src/renderer/categories-settings.js."
  },
  "settings.categories.jevError.key": {
    "message": "TypeSafe didn't accept the API key. Check it, or turn Jev off.",
    "description": "Settings > Inbox categories: error under the Jev switch when the key is refused. src/renderer/categories-settings.js."
  },
  "settings.categories.jevError.busy": {
    "message": "TypeSafe is busy. Rukoo tries again in a few minutes and sorts by rules meanwhile.",
    "description": "Settings > Inbox categories: error under the Jev switch when TypeSafe is busy. src/renderer/categories-settings.js."
  },
  "settings.categories.jevError.network": {
    "message": "Can't reach TypeSafe. Rukoo tries again in a few minutes and sorts by rules meanwhile.",
    "description": "Settings > Inbox categories: error under the Jev switch when TypeSafe can't be reached. src/renderer/categories-settings.js."
  },
  "settings.categories.again": {
    "message": "Sort the inbox again",
    "description": "Settings > Inbox categories: row that sorts the whole inbox again. src/renderer/categories-settings.js."
  },
  "settings.categories.againHelp": {
    "message": "From the start, apart from the emails you moved yourself",
    "description": "Settings > Inbox categories: description of the sort again row. src/renderer/categories-settings.js."
  },
  "settings.categories.againDone": {
    "message": {"one": "Sorted {count} email", "other": "Sorted {count} emails"},
    "description": "Settings > Inbox categories: toast after sorting again. {count}: emails sorted. src/renderer/categories-settings.js."
  },
  "settings.categories.senders": {
    "message": "Senders with a rule",
    "description": "Settings > Inbox categories: group title above the list of sender rules. src/renderer/categories-settings.js."
  },
  "settings.categories.senderHelp": {
    "message": "Goes to {name}. Choose to remove the rule.",
    "description": "Settings > Inbox categories: description of a sender rule row. {name}: the category. src/renderer/categories-settings.js."
  },
  "settings.categories.senderRemove": {
    "message": "Remove the rule for {sender}?",
    "description": "Settings > Inbox categories: title of the confirmation before removing a sender rule. src/renderer/categories-settings.js."
  },
  "settings.categories.senderRemoveHelp": {
    "message": "New mail from this sender is sorted like any other mail again.",
    "description": "Settings > Inbox categories: text of the confirmation before removing a sender rule. src/renderer/categories-settings.js."
  },
  "settings.categories.sync": {
    "message": "Your categories, the category each email got and your corrections are in your Rukoo account, so your other devices show the same.",
    "description": "Settings > Inbox categories: note when signed in to a Rukoo account. src/renderer/categories-settings.js."
  },
  "settings.categories.syncLocal": {
    "message": "These stay on this computer. Sign in to a Rukoo account to see the same categories on your other devices.",
    "description": "Settings > Inbox categories: note when not signed in to a Rukoo account. src/renderer/categories-settings.js."
  },
  "settings.rukoo.typesafe": {
    "message": "TypeSafe, Inc.",
    "description": "Settings > Rukoo account > What's stored and shared: name of the party that sorts mail with Jev. src/renderer/rukoo-account.js (privacyPage)."
  },
  "settings.rukoo.typesafeHelp": {
    "message": "Sorts your inbox with Jev, because you turned it on on this computer. Gets the sender, subject and first 1,000 characters of each new inbox email, in the United States. Its own terms apply to what it keeps.",
    "description": "Settings > Rukoo account > What's stored and shared: what TypeSafe handles when Jev is on. src/renderer/rukoo-account.js (privacyPage)."
  },
  "whatsNew.welcome.title": {
    "message": "What's new in Rukoo 2.1",
    "description": "\"What's new in Rukoo 2.1\" guide: title of the first step. src/renderer/whats-new.js."
  },
  "whatsNew.welcome.lead": {
    "message": "Rukoo can now sort your inbox into categories, the way Gmail does with its tabs, but for every account you use. Your mail itself doesn't change.",
    "description": "\"What's new in Rukoo 2.1\" guide: introduction on the first step. src/renderer/whats-new.js."
  },
  "whatsNew.welcome.categoriesTitle": {
    "message": "Inbox categories",
    "description": "\"What's new in Rukoo 2.1\" guide: first feature on the first step. src/renderer/whats-new.js."
  },
  "whatsNew.welcome.categoriesText": {
    "message": "Needs you, To read, Money, Updates and Promotions, under Inbox in the sidebar. Rukoo never moves or labels mail, so other mail apps see your inbox as before.",
    "description": "\"What's new in Rukoo 2.1\" guide: text of the first feature. src/renderer/whats-new.js."
  },
  "whatsNew.welcome.syncTitle": {
    "message": "The same on every device",
    "description": "\"What's new in Rukoo 2.1\" guide: second feature on the first step. src/renderer/whats-new.js."
  },
  "whatsNew.welcome.syncText": {
    "message": "A free Rukoo account keeps your categories and corrections the same on your computers and phone. It is end-to-end encrypted.",
    "description": "\"What's new in Rukoo 2.1\" guide: text of the second feature. src/renderer/whats-new.js."
  },
  "whatsNew.welcome.jevTitle": {
    "message": "Smarter sorting, if you want it",
    "description": "\"What's new in Rukoo 2.1\" guide: third feature on the first step. src/renderer/whats-new.js."
  },
  "whatsNew.welcome.jevText": {
    "message": "With your own TypeSafe key, Jev sorts the mail Rukoo's rules aren't sure about.",
    "description": "\"What's new in Rukoo 2.1\" guide: text of the third feature. src/renderer/whats-new.js."
  },
  "whatsNew.welcome.calendarTitle": {
    "message": "Calendar and booking page",
    "description": "\"What's new in Rukoo 2.1\" guide: fourth feature on the first step, the calendar's additions and the booking page. src/renderer/whats-new.js."
  },
  "whatsNew.welcome.calendarText": {
    "message": "A month view, repeating events and reminders. On your booking page, people and their AI assistants pick a time with you.",
    "description": "\"What's new in Rukoo 2.1\" guide: text of the fourth feature, about the calendar and the booking page (Settings → Booking page). src/renderer/whats-new.js."
  },
  "whatsNew.notNow": {
    "message": "Not now",
    "description": "\"What's new in Rukoo 2.1\" guide: button that closes the guide on the first step. src/renderer/whats-new.js."
  },
  "whatsNew.start": {
    "message": "Set it up",
    "description": "\"What's new in Rukoo 2.1\" guide: button that starts the guide on the first step. src/renderer/whats-new.js."
  },
  "whatsNew.next": {
    "message": "Next",
    "description": "\"What's new in Rukoo 2.1\" guide: button to the next step. src/renderer/whats-new.js."
  },
  "whatsNew.step": {
    "message": "Step {step} of {count}",
    "description": "\"What's new\" guide: accessible name of the progress dots. {step}: this step, {count}: all steps. src/renderer/whats-new.js."
  },
  "whatsNew.categories.title": {
    "message": "Choose your categories",
    "description": "\"What's new in Rukoo 2.1\" guide: title of the categories step. src/renderer/whats-new.js."
  },
  "whatsNew.categories.lead": {
    "message": "Rukoo starts with these five. Turn off the ones you don't need; you can rename them, change their colors or add your own later in Settings.",
    "description": "\"What's new in Rukoo 2.1\" guide: introduction on the categories step for someone without categories yet. src/renderer/whats-new.js."
  },
  "whatsNew.categories.leadExisting": {
    "message": "These are your categories. Change them in Settings > Inbox categories.",
    "description": "\"What's new in Rukoo 2.1\" guide: introduction on the categories step for someone who already has categories. src/renderer/whats-new.js."
  },
  "whatsNew.categories.unsureHelp": {
    "message": "Mail the rules can't place goes here.",
    "description": "\"What's new in Rukoo 2.1\" guide: help under \"When Rukoo isn't sure\" on the categories step. src/renderer/whats-new.js."
  },
  "whatsNew.categories.keepOne": {
    "message": "Keep at least one category.",
    "description": "\"What's new in Rukoo 2.1\" guide: toast when every category is turned off. src/renderer/whats-new.js."
  },
  "whatsNew.account.title": {
    "message": "Keep them the same on every device",
    "description": "\"What's new in Rukoo 2.1\" guide: title of the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.lead": {
    "message": "Your categories, the category each email got and your corrections live in a Rukoo account, so your phone and other computers sort the same way.",
    "description": "\"What's new in Rukoo 2.1\" guide: introduction on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.e2e": {
    "message": "End-to-end encrypted.",
    "description": "\"What's new in Rukoo 2.1\" guide: start of the note about encryption on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.e2eText": {
    "message": "Rukoo encrypts it on this device with a key only your devices have. The server never sees your mail, senders or category names.",
    "description": "\"What's new in Rukoo 2.1\" guide: rest of the note about encryption on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.email": {
    "message": "Email address",
    "description": "\"What's new in Rukoo 2.1\" guide: placeholder of the email field on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.password": {
    "message": "Password, at least 12 characters",
    "description": "\"What's new in Rukoo 2.1\" guide: placeholder of the new password field on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.passwordSignIn": {
    "message": "Password",
    "description": "\"What's new in Rukoo 2.1\" guide: placeholder of the password field when signing in on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.signedInHelp": {
    "message": "Your categories sync with your other devices.",
    "description": "\"What's new in Rukoo 2.1\" guide: shown under \"Signed in as\" on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.create": {
    "message": "Create a Rukoo account",
    "description": "\"What's new in Rukoo 2.1\" guide: first choice on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.createHint": {
    "message": "Free. While sign-ups are limited, you need an invite code.",
    "description": "\"What's new in Rukoo 2.1\" guide: hint under the first choice on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.signIn": {
    "message": "I already have an account",
    "description": "\"What's new in Rukoo 2.1\" guide: second choice on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.signInHint": {
    "message": "Sign in with the account you made on another device.",
    "description": "\"What's new in Rukoo 2.1\" guide: hint under the second choice on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.local": {
    "message": "Only on this computer",
    "description": "\"What's new in Rukoo 2.1\" guide: third choice on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.localHint": {
    "message": "No account. You can make one later in Settings > Rukoo account.",
    "description": "\"What's new in Rukoo 2.1\" guide: hint under the third choice on the account step. src/renderer/whats-new.js."
  },
  "whatsNew.account.saveKey": {
    "message": "Tick the box once you saved the key.",
    "description": "\"What's new in Rukoo 2.1\" guide: toast on the recovery key step when the box is not ticked. src/renderer/whats-new.js."
  },
  "whatsNew.jev.title": {
    "message": "Smarter sorting with Jev",
    "description": "\"What's new in Rukoo 2.1\" guide: title of the Jev step. src/renderer/whats-new.js."
  },
  "whatsNew.jev.lead": {
    "message": "Rukoo sorts by the sender and the mail's headers, and on Gmail accounts by Gmail's own tabs. For mail it isn't sure about, Jev, a decision model by TypeSafe, can read the email and choose from your categories.",
    "description": "\"What's new in Rukoo 2.1\" guide: introduction on the Jev step. src/renderer/whats-new.js."
  },
  "whatsNew.jev.rules": {
    "message": "Rules only",
    "description": "\"What's new in Rukoo 2.1\" guide: first choice on the Jev step. src/renderer/whats-new.js."
  },
  "whatsNew.jev.rulesHint": {
    "message": "Nothing leaves this computer. Mail Rukoo isn't sure about goes to {name}.",
    "description": "\"What's new in Rukoo 2.1\" guide: hint under \"Rules only\". {name}: the category for unsure mail. src/renderer/whats-new.js."
  },
  "whatsNew.jev.use": {
    "message": "Use Jev with my TypeSafe key",
    "description": "\"What's new in Rukoo 2.1\" guide: second choice on the Jev step. src/renderer/whats-new.js."
  },
  "whatsNew.jev.useHint": {
    "message": "Optional. TypeSafe charges you for what you use.",
    "description": "\"What's new in Rukoo 2.1\" guide: hint under the second choice on the Jev step. src/renderer/whats-new.js."
  },
  "whatsNew.jev.keySaved": {
    "message": "Key saved; enter a new one to replace it",
    "description": "\"What's new in Rukoo 2.1\" guide: placeholder of the key field when a key is already saved. src/renderer/whats-new.js."
  },
  "whatsNew.jev.needKey": {
    "message": "Enter your TypeSafe key, or choose Rules only.",
    "description": "\"What's new in Rukoo 2.1\" guide: toast when Jev is chosen without a key. src/renderer/whats-new.js."
  },
  "whatsNew.done.title": {
    "message": "You're all set",
    "description": "\"What's new in Rukoo 2.1\" guide: title of the last step. src/renderer/whats-new.js."
  },
  "whatsNew.done.lead": {
    "message": "Rukoo sorts your inbox now, and new mail as it arrives. Move an email to another category any time; Rukoo can remember that for its sender.",
    "description": "\"What's new in Rukoo 2.1\" guide: introduction on the last step. src/renderer/whats-new.js."
  },
  "whatsNew.done.leadExisting": {
    "message": "Your inbox is already sorted. Rukoo keeps doing it as new mail arrives.",
    "description": "\"What's new in Rukoo 2.1\" guide: introduction on the last step when categories were already on. src/renderer/whats-new.js."
  },
  "whatsNew.done.categories": {
    "message": {"one": "{count} category", "other": "{count} categories"},
    "description": "\"What's new\" guide, last step: how many categories. {count}: the number. src/renderer/whats-new.js."
  },
  "whatsNew.done.synced": {
    "message": "Synced with {email}",
    "description": "\"What's new in Rukoo 2.1\" guide: summary line when signed in. {email}: the account email address. src/renderer/whats-new.js."
  },
  "whatsNew.done.syncedHelp": {
    "message": "Your other devices show the same categories.",
    "description": "\"What's new in Rukoo 2.1\" guide: help under the synced summary line. src/renderer/whats-new.js."
  },
  "whatsNew.done.local": {
    "message": "Only on this computer",
    "description": "\"What's new in Rukoo 2.1\" guide: summary line without an account. src/renderer/whats-new.js."
  },
  "whatsNew.done.localHelp": {
    "message": "Make an account any time in Settings > Rukoo account.",
    "description": "\"What's new in Rukoo 2.1\" guide: help under the summary line without an account. src/renderer/whats-new.js."
  },
  "whatsNew.done.jev": {
    "message": "Jev sorts what the rules aren't sure about",
    "description": "\"What's new in Rukoo 2.1\" guide: summary line with Jev on. src/renderer/whats-new.js."
  },
  "whatsNew.done.jevHelp": {
    "message": "With your TypeSafe key.",
    "description": "\"What's new in Rukoo 2.1\" guide: help under the Jev summary line. src/renderer/whats-new.js."
  },
  "whatsNew.done.rules": {
    "message": "Sorted by rules",
    "description": "\"What's new in Rukoo 2.1\" guide: summary line without Jev. src/renderer/whats-new.js."
  },
  "whatsNew.done.sort": {
    "message": "Sort my inbox",
    "description": "\"What's new in Rukoo 2.1\" guide: button that finishes the guide and sorts the inbox. src/renderer/whats-new.js."
  },
  "whatsNew.done.close": {
    "message": "Done",
    "description": "\"What's new in Rukoo 2.1\" guide: button that closes the guide when categories were already on. src/renderer/whats-new.js."
  },
  "whatsNew.settingsRow": {
    "message": "What's new in Rukoo 2.1",
    "description": "Settings > About: the row that opens the \"What's new\" guide again. src/renderer/settings.js."
  },
  "whatsNew.settingsHelp": {
    "message": "Inbox categories, the Rukoo account and Jev, step by step",
    "description": "Settings > About: description of the \"What's new\" row. src/renderer/settings.js."
  },
  "updates.button.label": {
    "message": "Update to {version}",
    "description": "Sidebar: small button above the sync status when a newer Rukoo is on GitHub. {version}: e.g. 2.1.0. src/renderer/app.js (updateButton)."
  },
  "updates.button.title": {
    "message": "Rukoo {version} is available. Opens its download page.",
    "description": "Sidebar: tooltip of the update button. {version}: e.g. 2.1.0. src/renderer/app.js (updateButton)."
  },
  "settings.updates.title": {
    "message": "Check for updates",
    "description": "Settings > About: switch for the update check. src/renderer/settings.js."
  },
  "settings.updates.help": {
    "message": "Rukoo asks GitHub for new releases every few hours. On Windows and in the AppImage it downloads them too. GitHub sees your IP address.",
    "description": "Settings > About: description of the update check switch. src/renderer/settings.js."
  },
  "settings.updates.checkNow": {
    "message": "Check now",
    "description": "Settings > About: row that checks for a newer Rukoo right away. src/renderer/settings.js."
  },
  "settings.updates.available": {
    "message": "Rukoo {version} is available",
    "description": "Settings > About: value of Check now, and a toast, when a newer Rukoo exists. {version}: e.g. 2.1.0. src/renderer/settings.js."
  },
  "settings.updates.upToDate": {
    "message": "Rukoo is up to date",
    "description": "Settings > About: value of Check now, and a toast, when no newer Rukoo exists. src/renderer/settings.js."
  },
  "settings.updates.failed": {
    "message": "Couldn't reach GitHub. Try again later.",
    "description": "Settings > About: toast when the update check failed. src/renderer/settings.js."
  },
  "updates.button.restart": {
    "message": "Restart to update",
    "description": "Sidebar: the update button once a newer Rukoo has downloaded; restarts into it. src/renderer/app.js (updateButton)."
  },
  "updates.button.restartTitle": {
    "message": "Rukoo {version} has downloaded. Restarts Rukoo to install it.",
    "description": "Sidebar: tooltip of the Restart to update button. {version}: e.g. 2.1.1. src/renderer/app.js (updateButton)."
  },
  "settings.updates.channel": {
    "message": "Update channel",
    "description": "Settings > About: row and picker title for which releases the update check offers. src/renderer/settings.js."
  },
  "settings.updates.stable": {
    "message": "Stable",
    "description": "Settings > About > Update channel: option for tested releases. src/renderer/settings.js (CHANNELS)."
  },
  "settings.updates.stableHint": {
    "message": "Tested releases, about every two weeks. Coming from Nightly, Rukoo waits for the next one.",
    "description": "Settings > About > Update channel: explanation under Stable. Switching back from Nightly never installs an older version. src/renderer/settings.js (CHANNELS)."
  },
  "settings.updates.nightly": {
    "message": "Nightly",
    "description": "Settings > About > Update channel: option for daily development builds. Product term; keep it short. src/renderer/settings.js (CHANNELS)."
  },
  "settings.updates.nightlyHint": {
    "message": "The newest changes, built each day something changed. Can contain bugs.",
    "description": "Settings > About > Update channel: explanation under Nightly. src/renderer/settings.js (CHANNELS)."
  },
  "settings.updates.downloading": {
    "message": "Downloading Rukoo {version} ({percent}%)",
    "description": "Settings > About: value of Check now while an update downloads. {version}: e.g. 2.1.1; {percent}: 0 to 100. src/renderer/settings.js (updatesValue)."
  },
  "settings.updates.ready": {
    "message": "Rukoo {version} installs when you quit",
    "description": "Settings > About: value of Check now, and a toast, once an update has downloaded. {version}: e.g. 2.1.1. src/renderer/settings.js (updatesValue)."
  },
  "settings.updates.restart": {
    "message": "Restart to update",
    "description": "Settings > About: row that restarts Rukoo into a downloaded update. src/renderer/settings.js."
  },
  "settings.updates.restartHelp": {
    "message": "Closes Rukoo, installs the new version and opens it again.",
    "description": "Settings > About: description of the Restart to update row. src/renderer/settings.js."
  },
  "settings.keyboard.title": {
    "message": "Keyboard shortcuts",
    "description": "Settings > Keyboard shortcuts: page title and row on the Settings overview. src/renderer/settings.js."
  },
  "settings.keyboard.rowHelp": {
    "message": "Use the shortcuts of the mail app you know",
    "description": "Settings > Keyboard shortcuts: description of the row on the Settings overview. src/renderer/settings.js."
  },
  "settings.keyboard.intro": {
    "message": "Pick the mail app whose shortcuts you know. Rukoo's own are the default.",
    "description": "Settings > Keyboard shortcuts: text above the list of schemes. src/renderer/settings.js."
  },
  "settings.keyboard.list": {
    "message": "Shortcuts in {scheme}",
    "description": "Settings > Keyboard shortcuts: group title above the shortcuts of the chosen scheme. {scheme}: e.g. Gmail. src/renderer/settings.js."
  },
  "settings.keyboard.fixed": {
    "message": "In every scheme, Ctrl+J opens the chat, Ctrl+Z undoes and Esc closes or clears.",
    "description": "Settings > Keyboard shortcuts: note under the list of shortcuts. src/renderer/settings.js."
  },
  "settings.keyboard.actions.compose": {
    "message": "New message",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.reply": {
    "message": "Reply",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.replyAll": {
    "message": "Reply all",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.forward": {
    "message": "Forward",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.archive": {
    "message": "Archive",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.delete": {
    "message": "Delete",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.read": {
    "message": "Mark as read",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.unread": {
    "message": "Mark as unread",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.toggleRead": {
    "message": "Mark as read or unread",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.star": {
    "message": "Star",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.move": {
    "message": "Move to folder",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.category": {
    "message": "Move to category",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.next": {
    "message": "Next email",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.previous": {
    "message": "Previous email",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.search": {
    "message": "Search",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.sync": {
    "message": "Sync",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.print": {
    "message": "Print",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.keyboard.actions.selectAll": {
    "message": "Select all",
    "description": "Settings > Keyboard shortcuts: name of an action in the list of shortcuts, and in a scheme's example line. src/renderer/settings.js."
  },
  "settings.swipe.title": {
    "message": "Swipe and quick actions",
    "description": "Settings > Swipe and quick actions: page title and row on the Settings overview. src/renderer/settings.js."
  },
  "settings.swipe.rowHelp": {
    "message": "What a swipe or the buttons on an email do",
    "description": "Settings > Swipe and quick actions: description of the row on the Settings overview. src/renderer/settings.js."
  },
  "settings.swipe.touch": {
    "message": "Swipe on touch screens",
    "description": "Settings > Swipe and quick actions: switch for swiping list rows. src/renderer/settings.js."
  },
  "settings.swipe.touchHelp": {
    "message": "Swipe an email in the list with a finger or pen.",
    "description": "Settings > Swipe and quick actions: description of the swipe switch. src/renderer/settings.js."
  },
  "settings.swipe.right": {
    "message": "Swipe right",
    "description": "Settings > Swipe and quick actions: row and dialog title: the action of a swipe to the right. src/renderer/settings.js."
  },
  "settings.swipe.left": {
    "message": "Swipe left",
    "description": "Settings > Swipe and quick actions: row and dialog title: the action of a swipe to the left. src/renderer/settings.js."
  },
  "settings.swipe.quick": {
    "message": "Quick actions",
    "description": "Settings > Swipe and quick actions: group title above the quick action switches. src/renderer/settings.js."
  },
  "settings.swipe.quickHelp": {
    "message": "The buttons on an email under the mouse, at most four.",
    "description": "Settings > Swipe and quick actions: text under the quick action switches. src/renderer/settings.js."
  },
  "settings.swipe.quickMax": {
    "message": "Choose at most four quick actions.",
    "description": "Settings > Swipe and quick actions: toast when a fifth quick action is turned on. src/renderer/settings.js."
  },
  "settings.actions.none": {
    "message": "Nothing",
    "description": "Settings > Swipe and quick actions: an action for a swipe or a quick action button. src/renderer/settings.js."
  },
  "settings.actions.read": {
    "message": "Mark as read or unread",
    "description": "Settings > Swipe and quick actions: an action for a swipe or a quick action button. src/renderer/settings.js."
  },
  "settings.actions.archive": {
    "message": "Archive",
    "description": "Settings > Swipe and quick actions: an action for a swipe or a quick action button. src/renderer/settings.js."
  },
  "settings.actions.delete": {
    "message": "Delete",
    "description": "Settings > Swipe and quick actions: an action for a swipe or a quick action button. src/renderer/settings.js."
  },
  "settings.actions.star": {
    "message": "Star",
    "description": "Settings > Swipe and quick actions: an action for a swipe or a quick action button. src/renderer/settings.js."
  },
  "settings.actions.move": {
    "message": "Move to folder",
    "description": "Settings > Swipe and quick actions: an action for a swipe or a quick action button. src/renderer/settings.js."
  },
  "settings.actions.category": {
    "message": "Move to category",
    "description": "Settings > Swipe and quick actions: an action for a swipe or a quick action button. src/renderer/settings.js."
  },
  "mailbox.selection.all": {
    "message": "Select all",
    "description": "List toolbar while selecting: tooltip of the checkbox that selects every email shown, with its shortcut. src/renderer/app.js (renderListTools)."
  },
  "categories.activity.sorting": {
    "message": "Sorting your inbox",
    "description": "Sidebar, sorting status and its panel: line above the sync status while the rules sort a lot of email. src/renderer/sort-activity.js."
  },
  "categories.activity.sortingJev": {
    "message": "Jev is sorting",
    "description": "Sidebar, sorting status and its panel: line above the sync status while Jev sorts, followed by a count such as 12/48. src/renderer/sort-activity.js."
  },
  "categories.activity.sorted": {
    "message": {"one": "Sorted {count} email", "other": "Sorted {count} emails"},
    "description": "Sidebar, sorting status and its panel: line above the sync status, and in the panel, after a sort someone asked for. {count}: emails sorted. src/renderer/sort-activity.js."
  },
  "categories.activity.nothing": {
    "message": "Nothing new to sort",
    "description": "Sidebar, sorting status and its panel: line above the sync status after a sort that found nothing to sort. src/renderer/sort-activity.js."
  },
  "categories.activity.waitingUntil": {
    "message": "Jev waits until {time}",
    "description": "Sidebar, sorting status and its panel: line above the sync status while TypeSafe is busy or out of reach. {time}: when Rukoo tries again, such as 14:32. src/renderer/sort-activity.js."
  },
  "categories.activity.waitingTitle": {
    "message": "Jev is waiting",
    "description": "Sidebar, sorting status and its panel: panel title, and the line above the sync status, while TypeSafe is busy or out of reach. src/renderer/sort-activity.js."
  },
  "categories.activity.keyRefused": {
    "message": "Jev can't use your key",
    "description": "Sidebar, sorting status and its panel: line above the sync status and panel title when TypeSafe refused the API key. src/renderer/sort-activity.js."
  },
  "categories.activity.show": {
    "message": "What Jev is doing",
    "description": "Sidebar, sorting status and its panel: tooltip of the line above the sync status, and the accessible name of its panel. src/renderer/sort-activity.js."
  },
  "categories.activity.titleJev": {
    "message": "Jev is sorting your inbox",
    "description": "Sidebar, sorting status and its panel: panel title while Jev sorts. src/renderer/sort-activity.js."
  },
  "categories.activity.titleSorting": {
    "message": "Sorting your inbox",
    "description": "Sidebar, sorting status and its panel: panel title while the rules sort, without Jev. src/renderer/sort-activity.js."
  },
  "categories.activity.titleDone": {
    "message": "Your inbox is sorted",
    "description": "Sidebar, sorting status and its panel: panel title when nothing is being sorted. src/renderer/sort-activity.js."
  },
  "categories.activity.progress": {
    "message": "{done} of {total} sorted",
    "description": "Sidebar, sorting status and its panel: panel, under the title while sorting. {done}: emails sorted so far, {total}: emails in this sort. src/renderer/sort-activity.js."
  },
  "categories.activity.lessThanMinute": {
    "message": "less than a minute left",
    "description": "Sidebar, sorting status and its panel: panel, after the progress while sorting. src/renderer/sort-activity.js."
  },
  "categories.activity.minutesLeft": {
    "message": {"one": "about {count} minute left", "other": "about {count} minutes left"},
    "description": "Sidebar, sorting status and its panel: panel, after the progress while sorting. {count}: minutes. src/renderer/sort-activity.js."
  },
  "categories.activity.busy": {
    "message": "TypeSafe is busy.",
    "description": "Sidebar, sorting status and its panel: panel text while TypeSafe is busy. src/renderer/sort-activity.js."
  },
  "categories.activity.network": {
    "message": "Rukoo can't reach TypeSafe.",
    "description": "Sidebar, sorting status and its panel: panel text while TypeSafe is out of reach. src/renderer/sort-activity.js."
  },
  "categories.activity.retryAt": {
    "message": "Rukoo tries again at {time}.",
    "description": "Sidebar, sorting status and its panel: panel text after why Jev waits. {time}: such as 14:32. src/renderer/sort-activity.js."
  },
  "categories.activity.keyText": {
    "message": "TypeSafe didn't accept your API key. Check it in Settings, or turn Jev off.",
    "description": "Sidebar, sorting status and its panel: panel text when TypeSafe refused the API key. src/renderer/sort-activity.js."
  },
  "categories.activity.waitingCount": {
    "message": {"one": "{count} email waits for Jev.", "other": "{count} emails wait for Jev."},
    "description": "Sidebar, sorting status and its panel: panel text while Jev waits. {count}: emails. src/renderer/sort-activity.js."
  },
  "categories.activity.newMailJev": {
    "message": "Jev sorts new email as it comes in.",
    "description": "Sidebar, sorting status and its panel: panel text when sorting is done, with Jev on. src/renderer/sort-activity.js."
  },
  "categories.activity.newMail": {
    "message": "Rukoo sorts new email as it comes in.",
    "description": "Sidebar, sorting status and its panel: panel text when sorting is done, without Jev. src/renderer/sort-activity.js."
  },
  "categories.activity.recent": {
    "message": "Just sorted",
    "description": "Sidebar, sorting status and its panel: panel, heading above Jev's last decisions. src/renderer/sort-activity.js."
  },
  "categories.activity.sure": {
    "message": "{percent}% sure",
    "description": "Sidebar, sorting status and its panel: panel, after the sender of an email Jev sorted. {percent}: Jev's confidence. src/renderer/sort-activity.js."
  },
  "categories.activity.unsure": {
    "message": "not sure it's {name} ({percent}%)",
    "description": "Sidebar, sorting status and its panel: panel, after the sender of an email Jev wasn't sure about, which went to the category for unsure mail. {name}: the category Jev leaned to, {percent}: its confidence. src/renderer/sort-activity.js."
  },
  "categories.activity.meanwhile": {
    "message": "Emails Jev hasn't reached yet stay where they are for now.",
    "description": "Sidebar, sorting status and its panel: panel note while Jev sorts or waits. src/renderer/sort-activity.js."
  },
  "categories.activity.settings": {
    "message": "Settings",
    "description": "Sidebar, sorting status and its panel: panel button that opens Settings > Inbox categories. src/renderer/sort-activity.js."
  },
  "categories.activity.checkKey": {
    "message": "Check the key",
    "description": "Sidebar, sorting status and its panel: panel button that opens Settings > Inbox categories when the key was refused. src/renderer/sort-activity.js."
  },
  "meeting.title": {
    "message": "Meetings",
    "description": "Accessible name of the meeting view, the third mode next to Mail and Calendar. src/renderer/app.js (renderShell)"
  },
  "meeting.mode.label": {
    "message": "Meet",
    "description": "Sidebar switch at the top: the segment that shows Rukoo Voice meetings. Short; it shares a narrow sidebar with the Mail and Calendar segments. src/renderer/app.js (renderModeSwitch)"
  },
  "meeting.mode.shortcut": {
    "message": "Meet (Ctrl+3)",
    "description": "Tooltip of the Meet segment in the sidebar switch, with its keyboard shortcut. src/renderer/app.js (renderModeSwitch)"
  },
  "meeting.untitled": {
    "message": "Meeting at {time}",
    "description": "Meeting view, sidebar list and title bar: title of a meeting without a title or linked event; time is when it started, such as 09:31. src/renderer/meeting.js (titleOf)"
  },
  "meeting.untitledNoTime": {
    "message": "Meeting",
    "description": "Meeting view and chat panel: title of a meeting without a title, a linked event or a start time yet. src/renderer/meeting.js (titleOf)"
  },
  "meeting.errors.offline": {
    "message": "Rukoo Voice isn't running, or Rukoo can't reach it.",
    "description": "Meeting view toast: a command to Rukoo Voice failed because it is not connected (error code offline). src/renderer/meeting.js (voiceError)"
  },
  "meeting.errors.timeout": {
    "message": "Rukoo Voice didn't answer. Try again.",
    "description": "Meeting view toast: Rukoo Voice did not answer a command in time (timeout). src/renderer/meeting.js (voiceError)"
  },
  "meeting.errors.busy": {
    "message": "Rukoo Voice is already recording a meeting.",
    "description": "Meeting view toast: Record was clicked while another meeting records (busy). src/renderer/meeting.js (voiceError, notReadyText)"
  },
  "meeting.errors.not_found": {
    "message": "Rukoo Voice doesn't have this meeting.",
    "description": "Meeting view toast: Rukoo Voice no longer has the meeting a command named (not_found). src/renderer/meeting.js (voiceError)"
  },
  "meeting.errors.not_recording": {
    "message": "This meeting isn't recording.",
    "description": "Meeting view toast: Stop, Pause, Resume or Mark on a meeting that no longer records (not_recording). src/renderer/meeting.js (voiceError)"
  },
  "meeting.errors.not_available": {
    "message": "Rukoo Voice can't do that for this meeting.",
    "description": "Meeting view toast: Rukoo Voice cannot do the command for this meeting or in its version (not_available). src/renderer/meeting.js (voiceError)"
  },
  "meeting.errors.failed": {
    "message": "Rukoo Voice couldn't do that.",
    "description": "Meeting view toast: Rukoo Voice tried a command and failed, or another error. src/renderer/meeting.js (voiceError)"
  },
  "meeting.errors.bad_request": {
    "message": "Rukoo Voice didn't understand the request. Update Rukoo Voice or Rukoo.",
    "description": "Meeting view toast: Rukoo Voice refused a command it could not read (bad_request). src/renderer/meeting.js (voiceError)"
  },
  "meeting.errors.unknown_command": {
    "message": "This Rukoo Voice can't do that yet. Update Rukoo Voice.",
    "description": "Meeting view toast: an older Rukoo Voice does not know the command (unknown_command). src/renderer/meeting.js (voiceError)"
  },
  "meeting.voice.connecting": {
    "message": "Connecting to Rukoo Voice…",
    "description": "Meeting view empty state heading and toast while Rukoo connects to Rukoo Voice. src/renderer/meeting.js (emptyHtml, notReadyText)"
  },
  "meeting.voice.updateRukoo": {
    "message": "This Rukoo Voice is newer than Rukoo. Update Rukoo to record meetings.",
    "description": "Meeting view and Settings > Meetings: Rukoo Voice speaks only a newer bridge protocol than this Rukoo. src/renderer/meeting.js (notReadyText), src/renderer/meeting-settings.js"
  },
  "meeting.voice.updateVoice": {
    "message": "This Rukoo Voice is older than Rukoo. Update Rukoo Voice to record meetings.",
    "description": "Meeting view and Settings > Meetings: Rukoo Voice speaks only an older bridge protocol than this Rukoo. src/renderer/meeting.js (notReadyText), src/renderer/meeting-settings.js"
  },
  "meeting.voice.insecure": {
    "message": "Rukoo didn't connect: other accounts on this computer can read Rukoo Voice's connection file. Start Rukoo Voice again to fix it.",
    "description": "Meeting view and Settings > Meetings: the discovery file of Rukoo Voice is readable by other users, so Rukoo refuses it. src/renderer/meeting.js (notReadyText), src/renderer/meeting-settings.js"
  },
  "meeting.voice.absent": {
    "message": "Start Rukoo Voice to record meetings.",
    "description": "Meeting view toast and tooltip of Record while Rukoo Voice is not running. Rukoo cannot start it itself. src/renderer/meeting.js (notReadyText)"
  },
  "meeting.voice.recording": {
    "message": "Recording",
    "description": "Sidebar status line in meeting mode while Rukoo Voice records a meeting. src/renderer/meeting.js (footStatus)"
  },
  "meeting.voice.connected": {
    "message": "Voice connected",
    "description": "Sidebar status line in meeting mode while Rukoo is connected to Rukoo Voice. Short: it shares the sidebar foot with two buttons. src/renderer/meeting.js (footStatus)"
  },
  "meeting.voice.version": {
    "message": "Rukoo Voice {version}",
    "description": "Tooltip of the sidebar status line in meeting mode; version is the version of Rukoo Voice, such as 0.1.0. src/renderer/meeting.js (footStatus)"
  },
  "meeting.voice.connectingShort": {
    "message": "Connecting…",
    "description": "Sidebar status line in meeting mode while Rukoo connects to Rukoo Voice. src/renderer/meeting.js (footStatus)"
  },
  "meeting.voice.offShort": {
    "message": "Voice not running",
    "description": "Sidebar status line in meeting mode while Rukoo is not connected to Rukoo Voice. Short: it shares the sidebar foot with two buttons. src/renderer/meeting.js (footStatus)"
  },
  "meeting.empty.title": {
    "message": "Record a meeting",
    "description": "Meeting view empty state heading while Rukoo Voice is connected and no meeting is on screen. src/renderer/meeting.js (emptyHtml)"
  },
  "meeting.empty.startVoice": {
    "message": "Start Rukoo Voice to record meetings",
    "description": "Meeting view empty state heading while Rukoo Voice is not running. Rukoo cannot start it itself. src/renderer/meeting.js (emptyHtml)"
  },
  "meeting.empty.cantConnect": {
    "message": "Rukoo can't connect to Rukoo Voice",
    "description": "Meeting view empty state heading when Rukoo Voice runs but Rukoo refuses or cannot use the connection (an update is needed, or the connection file is not private). src/renderer/meeting.js (emptyHtml)"
  },
  "meeting.empty.body": {
    "message": "Rukoo Voice records your microphone and the call. The transcript shows here as people speak, next to your agent.",
    "description": "Meeting view empty state: what recording a meeting does. src/renderer/meeting.js (emptyHtml)"
  },
  "meeting.empty.calendarHint": {
    "message": "You can also record an event from its card in the calendar.",
    "description": "Meeting view empty state: hint under the Record a meeting button. src/renderer/meeting.js (emptyHtml)"
  },
  "meeting.empty.voiceHint": {
    "message": "Rukoo can't open Rukoo Voice for you. The meetings it records show here.",
    "description": "Meeting view empty state while Rukoo Voice is not running: Rukoo cannot launch it. src/renderer/meeting.js (emptyHtml)"
  },
  "meeting.actions.record": {
    "message": "Record a meeting",
    "description": "Meeting view: sidebar button and empty state button that start a recording in Rukoo Voice. src/renderer/meeting.js (sidebarHtml, emptyHtml)"
  },
  "meeting.actions.mark": {
    "message": "Mark",
    "description": "Meeting view header button while recording: marks this moment in the meeting. src/renderer/meeting.js (renderHead)"
  },
  "meeting.actions.pause": {
    "message": "Pause",
    "description": "Meeting view header button while recording: pauses the recording. src/renderer/meeting.js (renderHead)"
  },
  "meeting.actions.resume": {
    "message": "Resume",
    "description": "Meeting view header button while paused: records again. src/renderer/meeting.js (renderHead)"
  },
  "meeting.actions.stop": {
    "message": "Stop",
    "description": "Meeting view header button while recording: the first click asks Stop recording?, the second stops. src/renderer/meeting.js (renderHead)"
  },
  "meeting.screen.liveChoose": { "message": "Screen recording", "description": "Meeting header menu heading for changing screen recording during a live or paused meeting. src/renderer/meeting.js" },
  "meeting.screen.control.off": { "message": "Off", "description": "Live meeting screen menu: stop capturing the screen while audio continues. src/renderer/meeting.js" },
  "meeting.screen.control.frames": { "message": "Frames", "description": "Live meeting screen menu: record screen images and their text. src/renderer/meeting.js" },
  "meeting.screen.control.video": { "message": "Video", "description": "Live meeting screen menu: record video and screen frames. src/renderer/meeting.js" },
  "meeting.mark.screen.off": { "message": "Screen recording off", "description": "Transcript timestamp marker when the user disables screen capture. src/renderer/meeting.js" },
  "meeting.mark.screen.frames": { "message": "Screen recording on · Frames", "description": "Transcript timestamp marker when the user selects screen frames. src/renderer/meeting.js" },
  "meeting.mark.screen.video": { "message": "Screen recording on · Video", "description": "Transcript timestamp marker when the user selects screen video. src/renderer/meeting.js" },
  "agent.panel.meetingDeleted": { "message": "Meeting deleted", "description": "Context chip subtitle in a retained chat whose meeting was deleted; the conversation itself remains. src/renderer/agent/panel.js" },
  "meeting.actions.rename": { "message": "Rename meeting", "description": "Header edit button and sidebar menu action; opens the meeting title field. src/renderer/meeting.js" },
  "meeting.actions.more": { "message": "Meeting actions", "description": "Accessible label for the meeting header menu containing rename and delete. src/renderer/meeting.js" },
  "meeting.actions.delete": { "message": "Delete meeting", "description": "Destructive action in the meeting menu and delete confirmation. src/renderer/meeting.js" },
  "meeting.rename.invalid": { "message": "Use a title of 1 to 200 characters.", "description": "Validation error for the inline meeting title editor. src/renderer/meeting.js" },
  "meeting.delete.title": { "message": "Delete “{title}”?", "description": "Confirmation heading before permanently deleting a meeting; title is the meeting name. src/renderer/meeting.js" },
  "meeting.delete.body": { "message": "This permanently deletes the audio, screen recording, transcript, notes, speaker names and corrections from this computer. Synced copies are marked deleted when Voice next connects; encrypted bucket files follow the bucket's lifecycle rules. Chats about the meeting stay. This cannot be undone.", "description": "Explains local deletion, the queued bucket tombstone, retained encrypted bucket objects and retained chats. src/renderer/meeting.js" },
  "meeting.delete.done": { "message": "Meeting deleted", "description": "Toast after Voice and Rukoo accept a meeting deletion. src/renderer/meeting.js" },
  "meeting.delete.updateVoice": { "message": "Update Rukoo Voice to delete meetings.", "description": "Error when an older Voice does not implement meeting.delete. src/renderer/meeting.js" },
  "meeting.actions.stopConfirm": {
    "message": "Stop recording?",
    "description": "Meeting view header button after the first click on Stop; a second click within 3 seconds stops the recording. src/renderer/meeting.js (renderHead)"
  },
  "meeting.actions.marked": {
    "message": "Marked at {time}",
    "description": "Meeting view toast after Mark; time is the meeting time, such as 12:03. src/renderer/meeting.js (onClick)"
  },
  "meeting.actions.screen": {
    "message": "Screen",
    "description": "Meeting view header button while a meeting records the screen: opens the list of monitors to record. src/renderer/meeting.js (renderHead)"
  },
  "meeting.screen.menu": {
    "message": "Screen recording",
    "description": "Meeting view sidebar and empty state, calendar event card: accessible name of the small arrow beside Record that opens the choice of what to record of the screen. src/renderer/meeting.js (sidebarHtml, emptyHtml); src/renderer/calendar-event.js"
  },
  "meeting.screen.choose": {
    "message": "Choose what to record",
    "description": "Screen recording menu: heading above the choices of what Rukoo Voice records of the screen in the new meeting. src/renderer/meeting.js (screenMenu)"
  },
  "meeting.screen.default": {
    "message": "Voice's default ({mode})",
    "description": "Screen recording menu: the choice that records with Rukoo Voice's own setting; mode is one of meeting.screen.modeOff, modeFrames or modeVideo, in lower case. src/renderer/meeting.js (screenMenu)"
  },
  "meeting.screen.defaultPlain": {
    "message": "Voice's default",
    "description": "Screen recording menu: the same choice when Rukoo Voice didn't say what its setting is. src/renderer/meeting.js (screenMenu)"
  },
  "meeting.screen.off": {
    "message": "Audio only",
    "description": "Screen recording menu: records the meeting's audio and nothing of the screen. src/renderer/meeting.js (screenMenu)"
  },
  "meeting.screen.frames": {
    "message": "Audio and screen frames",
    "description": "Screen recording menu: records the audio and a still image of the screen each time it changes, with the text read from it. src/renderer/meeting.js (screenMenu)"
  },
  "meeting.screen.video": {
    "message": "Audio and screen video",
    "description": "Screen recording menu: records the audio, a video of the screen and the frames. src/renderer/meeting.js (screenMenu)"
  },
  "meeting.screen.modeOff": {
    "message": "no screen",
    "description": "What the screen recording is, inside meeting.screen.default and meeting.head.screen: nothing of the screen. Lower case, part of a sentence. src/renderer/meeting.js (screenMode)"
  },
  "meeting.screen.modeFrames": {
    "message": "frames",
    "description": "What the screen recording is, inside meeting.screen.default and meeting.head.screen: still images of the screen. Lower case, part of a sentence. src/renderer/meeting.js (screenMode)"
  },
  "meeting.screen.modeVideo": {
    "message": "video",
    "description": "What the screen recording is, inside meeting.screen.default and meeting.head.screen: a video of the screen. Lower case, part of a sentence. src/renderer/meeting.js (screenMode)"
  },
  "meeting.monitor.heading": {
    "message": "Record this monitor",
    "description": "Screen menu in the meeting view header: heading above the monitors Rukoo Voice can record. src/renderer/meeting.js (monitorMenu)"
  },
  "meeting.monitor.primary": {
    "message": "Main display",
    "description": "Screen menu: tag beside the monitor Windows uses as its main display. src/renderer/meeting.js (monitorMenu)"
  },
  "meeting.monitor.size": {
    "message": "{width} × {height}",
    "description": "Screen menu: a monitor's size in pixels under its name, such as 2560 × 1440. src/renderer/meeting.js (monitorMenu)"
  },
  "meeting.monitor.stopped": {
    "message": "Screen recording stopped",
    "description": "Screen menu: disabled line when Rukoo Voice no longer records the screen in this meeting, for example after a full disk. src/renderer/meeting.js (monitorMenu)"
  },
  "meeting.monitor.loading": {
    "message": "Asking Rukoo Voice…",
    "description": "Screen menu: shown while Rukoo asks Rukoo Voice which monitors there are. src/renderer/meeting.js (monitorMenu)"
  },
  "meeting.tabs.label": {
    "message": "What the meeting view shows",
    "description": "Meeting view in a narrow window: accessible name of the tabs that switch between transcript, notes and details. src/renderer/meeting.js (shellHtml)"
  },
  "meeting.tabs.transcript": {
    "message": "Transcript",
    "description": "Meeting view: tab in a narrow window, and accessible name of the transcript pane. src/renderer/meeting.js (shellHtml)"
  },
  "meeting.tabs.notes": {
    "message": "Notes",
    "description": "Meeting view tab in a narrow window: your notes. src/renderer/meeting.js (shellHtml)"
  },
  "meeting.tabs.details": {
    "message": "Details",
    "description": "Meeting view tab in a narrow window: the linked calendar event and the people. src/renderer/meeting.js (shellHtml)"
  },
  "meeting.transcript.label": {
    "message": "Transcript. Arrow keys move between lines; Enter corrects one.",
    "description": "Meeting view: accessible name of the transcript list, with its keyboard use. src/renderer/meeting.js (shellHtml)"
  },
  "meeting.transcript.latest": {
    "message": "Latest",
    "description": "Meeting view: button over the transcript that scrolls down to new lines after you scrolled up. src/renderer/meeting.js (shellHtml)"
  },
  "meeting.transcript.dictating": {
    "message": "Dictation",
    "description": "Meeting view: tag on a transcript line you dictated into another app during the meeting. src/renderer/meeting.js (passageHtml)"
  },
  "meeting.transcript.dictatingHelp": {
    "message": "You dictated this into another app. The call heard it, but it wasn't part of the conversation.",
    "description": "Meeting view: tooltip of the Dictation tag on a transcript line. src/renderer/meeting.js (passageHtml)"
  },
  "meeting.transcript.corrected": {
    "message": "Corrected by you",
    "description": "Meeting view: note under a transcript line whose text you corrected. src/renderer/meeting.js (passageHtml)"
  },
  "meeting.transcript.correctLabel": {
    "message": "Corrected text",
    "description": "Meeting view: accessible name of the field that corrects a transcript line. src/renderer/meeting.js (editPassage)"
  },
  "meeting.transcript.correctHint": {
    "message": "Enter saves, Esc cancels",
    "description": "Meeting view: hint beside Cancel and Save while correcting a transcript line. src/renderer/meeting.js (editPassage)"
  },
  "meeting.transcript.correct": {
    "message": "Correct the text",
    "description": "Meeting view: context menu item on a transcript line that opens it for correcting. src/renderer/meeting.js (onContextMenu)"
  },
  "meeting.transcript.restore": {
    "message": "Restore what was recognized",
    "description": "Meeting view: context menu item on a corrected transcript line that brings back the text Rukoo Voice recognized. src/renderer/meeting.js (onContextMenu)"
  },
  "meeting.event.title": {
    "message": "Calendar event",
    "description": "Meeting view side column: heading of the calendar event the meeting is linked to. src/renderer/meeting.js (shellHtml)"
  },
  "meeting.event.change": {
    "message": "Change",
    "description": "Meeting view side column: button that picks another calendar event for the meeting. src/renderer/meeting.js (renderEvent)"
  },
  "meeting.event.link": {
    "message": "Link",
    "description": "Meeting view side column: button that links a calendar event to a meeting without one. src/renderer/meeting.js (renderEvent)"
  },
  "meeting.event.none": {
    "message": "Not linked to an event",
    "description": "Meeting view side column: the meeting has no calendar event. src/renderer/meeting.js (renderEvent)"
  },
  "meeting.event.missing": {
    "message": "The linked event isn't in your calendar any more",
    "description": "Meeting view side column: the linked calendar event was deleted or is out of reach. src/renderer/meeting.js (renderEvent)"
  },
  "meeting.event.open": {
    "message": "Open in the calendar",
    "description": "Meeting view side column: tooltip of the linked event title, which opens it in the calendar. src/renderer/meeting.js (renderEvent)"
  },
  "meeting.event.around": {
    "message": "Events around the start",
    "description": "Meeting view: heading of the menu that links a calendar event; it lists events within 15 minutes of the start. src/renderer/meeting.js (eventMenu)"
  },
  "meeting.event.noneFound": {
    "message": "No events around the start",
    "description": "Meeting view: menu line when no calendar event lies within 15 minutes of the meeting start. src/renderer/meeting.js (eventMenu)"
  },
  "meeting.event.someFailed": {
    "message": "Some calendars couldn't be read",
    "description": "Meeting view: menu line when a calendar account failed while looking for events. src/renderer/meeting.js (eventMenu)"
  },
  "meeting.event.unlink": {
    "message": "Unlink the event",
    "description": "Meeting view: menu item that removes the link between the meeting and its calendar event. src/renderer/meeting.js (eventMenu)"
  },
  "meeting.speakers.title": {
    "message": "People",
    "description": "Meeting view side column: heading of the voices in the meeting, with their names. src/renderer/meeting.js (shellHtml)"
  },
  "meeting.speakers.you": {
    "message": "You",
    "description": "Meeting view: name of your own voice (the microphone) in the transcript and the people list. src/renderer/meeting.js (speakerName)"
  },
  "meeting.speakers.unknown": {
    "message": "Someone",
    "description": "Meeting view: name of a voice Rukoo Voice has no label for yet. src/renderer/meeting.js (speakerName)"
  },
  "meeting.speakers.numbered": {
    "message": "Speaker {n}",
    "description": "Meeting view: name of an unnamed voice on the call in the transcript, the people list and the name picker; n is the number Rukoo Voice gave the voice in the order it first heard it, such as 2. src/renderer/meeting.js (speakerLabel)"
  },
  "meeting.speakers.taken": {
    "message": "Already chosen for {name}",
    "description": "Meeting view name picker: second line under a guest who is already the name of another voice on the call; it can still be chosen. name is that other voice's name. src/renderer/meeting.js (speakerMenu)"
  },
  "meeting.speakers.autoCheck": {
    "message": "Guessed from the invitation; check it",
    "description": "Meeting view people list: second line under a name Rukoo filled in by itself when the call had one voice and the event one guest, shown once a second voice is on the call. src/renderer/meeting.js (renderPeople)"
  },
  "meeting.speakers.microphone": {
    "message": "Your microphone",
    "description": "Meeting view people list: second line under You. src/renderer/meeting.js (renderPeople)"
  },
  "meeting.speakers.was": {
    "message": "Was {label}",
    "description": "Meeting view people list: second line under a named voice; label is what Rukoo Voice called it, such as Speaker 1. src/renderer/meeting.js (renderPeople)"
  },
  "meeting.speakers.pick": {
    "message": "Choose who this is",
    "description": "Meeting view people list: second line under an unnamed voice when the linked event has guests to pick from. src/renderer/meeting.js (renderPeople)"
  },
  "meeting.speakers.nameIt": {
    "message": "Give this voice a name",
    "description": "Meeting view people list: second line under an unnamed voice without guests to pick from. src/renderer/meeting.js (renderPeople)"
  },
  "meeting.speakers.choose": {
    "message": "Choose a name for {name}",
    "description": "Meeting view people list: tooltip of a voice button that opens the name picker; name is its current name or label. src/renderer/meeting.js (renderPeople)"
  },
  "meeting.speakers.fromEvent": {
    "message": "Guests of {title}",
    "description": "Meeting view name picker: heading above the guests of the linked calendar event; title is the event title. src/renderer/meeting.js (speakerMenu)"
  },
  "meeting.speakers.guests": {
    "message": "Guests",
    "description": "Meeting view name picker: heading above the guests when the event title is unknown. src/renderer/meeting.js (speakerMenu)"
  },
  "meeting.speakers.someoneElse": {
    "message": "Someone else…",
    "description": "Meeting view name picker: item that asks for a name to type. src/renderer/meeting.js (speakerMenu)"
  },
  "meeting.speakers.clear": {
    "message": "Back to {label}",
    "description": "Meeting view name picker: item that forgets the name; label is what Rukoo Voice called the voice, such as Speaker 1. src/renderer/meeting.js (speakerMenu)"
  },
  "meeting.speakers.nameTitle": {
    "message": "Who is {label}?",
    "description": "Meeting view: title of the dialog that names a voice; label is what Rukoo Voice called it, such as Speaker 1. src/renderer/meeting.js (nameSpeaker)"
  },
  "meeting.speakers.namePlaceholder": {
    "message": "Name",
    "description": "Meeting view: placeholder of the name field in the dialog that names a voice. src/renderer/meeting.js (nameSpeaker)"
  },
  "meeting.speakers.none": {
    "message": "Voices show here once someone speaks.",
    "description": "Meeting view people list before Rukoo Voice reported any voice. src/renderer/meeting.js (renderPeople)"
  },
  "meeting.frame.visibility": {"message":"Screen","description":"Meeting transcript toggle in meeting.js. Shows or hides screen frames; remembered on this device."},
  "meeting.frame.group": {"message":"Show {count} more screen frames","description":"Meeting transcript button opening the folded frame strip; count is the number of additional frames."},
  "meeting.notes.heading": {"message":"Heading","description":"Meeting notes toolbar action applying a level two heading with the shared editor."},
  "meeting.notes.title": {
    "message": "Your notes",
    "description": "Meeting view side column: heading of your notes about the meeting. src/renderer/meeting.js (shellHtml)"
  },
  "meeting.notes.placeholder": {
    "message": "Notes for yourself. Your agent can read them too.",
    "description": "Meeting view: placeholder of the empty notes field. src/renderer/meeting.js (shellHtml)"
  },
  "meeting.notes.saved": {
    "message": "Saved",
    "description": "Meeting view: status beside the notes heading after Rukoo saved them. src/renderer/meeting.js (saveNotes)"
  },
  "meeting.notes.saving": {
    "message": "Saving…",
    "description": "Meeting view: status beside the notes heading while Rukoo saves them. src/renderer/meeting.js (saveNotes)"
  },
  "meeting.notes.failed": {
    "message": "Not saved",
    "description": "Meeting view: status beside the notes heading when saving failed; Rukoo tries again with the next change. src/renderer/meeting.js (saveNotes)"
  },
  "meeting.notes.tooLong": {
    "message": "Not saved: shorten the notes",
    "description": "Meeting notes status when serialized Markdown exceeds the storage limit. Saved notes remain unchanged; the pending editor text stays in memory so the user can shorten it. src/renderer/meeting.js (fillNotes, drainNotes)"
  },
  "meeting.state.recording": {
    "message": "Recording",
    "description": "Meeting view header: state pill while Rukoo Voice records the meeting. src/renderer/meeting.js (stateHtml)"
  },
  "meeting.state.paused": {
    "message": "Paused",
    "description": "Meeting view header: state pill while the recording is paused. src/renderer/meeting.js (stateHtml)"
  },
  "meeting.state.processing": {
    "message": "Finishing the transcript",
    "description": "Meeting view header: state pill after the recording stopped while Rukoo Voice still transcribes. src/renderer/meeting.js (stateHtml)"
  },
  "meeting.state.failed": {
    "message": "Transcript incomplete",
    "description": "Meeting view header: state pill when Rukoo Voice could not finish the transcript; the tooltip has its reason. src/renderer/meeting.js (stateHtml)"
  },
  "meeting.state.removed": {
    "message": "Deleted in Rukoo Voice",
    "description": "Meeting view header: state pill of a meeting Rukoo Voice deleted; your notes stay. src/renderer/meeting.js (stateHtml)"
  },
  "meeting.head.length": {
    "message": "How long the meeting ran",
    "description": "Meeting view header: tooltip of the meeting clock or length. src/renderer/meeting.js (renderHead)"
  },
  "meeting.head.hidden": {
    "message": "Hidden from screen recordings",
    "description": "Meeting view header while recording: Rukoo hides its windows from screen capture (Settings > Meetings). src/renderer/meeting.js (renderHead)"
  },
  "meeting.head.hiddenHelp": {
    "message": "Rukoo's windows stay out of screen recordings and screen shares until the meeting ends.",
    "description": "Meeting view header and title bar indicator: tooltip explaining Hidden from screen recordings. src/renderer/meeting.js (renderHead, renderIndicator)"
  },
  "meeting.head.screen": {
    "message": "Screen: {mode}",
    "description": "Meeting view header: chip saying what Rukoo Voice records of the screen in this meeting; mode is meeting.screen.modeFrames or modeVideo. src/renderer/meeting.js (renderHead)"
  },
  "meeting.head.visible": {
    "message": "Visible in screen recordings",
    "description": "Meeting view header while a meeting records the screen and Rukoo isn't hidden from screen capture: Rukoo can appear in the meeting's frames. src/renderer/meeting.js (renderHead)"
  },
  "meeting.head.visibleHelp": {
    "message": "Rukoo isn't hidden from screen recordings (Settings > Meetings), so this meeting's frames can show Rukoo.",
    "description": "Meeting view header: tooltip of Visible in screen recordings; names the setting that hides Rukoo. src/renderer/meeting.js (renderHead)"
  },
  "meeting.frame.alt": {
    "message": "The screen at {time}",
    "description": "Meeting view transcript: description of a screen frame image; time is the meeting time, such as 4:05. src/renderer/meeting.js (frameHtml)"
  },
  "meeting.frame.showText": {
    "message": "Text on screen",
    "description": "Meeting view transcript: button under a screen frame that shows all the text Rukoo Voice read from the screen. src/renderer/meeting.js (frameHtml)"
  },
  "meeting.frame.noText": {
    "message": "No text read from the screen",
    "description": "Meeting view transcript: beside a screen frame from which Rukoo Voice read no text. src/renderer/meeting.js (frameHtml)"
  },
  "meeting.frame.offline": {
    "message": "The image shows while Rukoo Voice runs.",
    "description": "Meeting view transcript: beside a screen frame whose image Rukoo can't get because Rukoo Voice isn't connected; Voice keeps the images. src/renderer/meeting.js (frameHtml)"
  },
  "meeting.frame.gone": {
    "message": "Rukoo Voice no longer has this image.",
    "description": "Meeting view transcript: beside a screen frame whose image Rukoo Voice removed, with the meeting's audio, under its retention setting. src/renderer/meeting.js (frameHtml)"
  },
  "meeting.notice.removed": {
    "message": "Rukoo Voice deleted this meeting and its transcript. Your notes stay here.",
    "description": "Meeting view: line under the header of a meeting Rukoo Voice deleted. src/renderer/meeting.js (renderNotice)"
  },
  "meeting.notice.offline": {
    "message": "Rukoo Voice isn't connected. The transcript continues here once it is.",
    "description": "Meeting view: line under the header of a meeting that records or is being transcribed while Rukoo Voice is not connected. src/renderer/meeting.js (renderNotice)"
  },
  "meeting.notice.failed": {
    "message": "Rukoo Voice couldn't transcribe all of this meeting. The audio is still on your computer.",
    "description": "Meeting view: line under the header when the transcript failed. src/renderer/meeting.js (renderNotice)"
  },
  "meeting.live.waiting": {
    "message": "Listening. The transcript follows as people speak.",
    "description": "Meeting view: line under an empty transcript while recording. src/renderer/meeting.js (renderLive)"
  },
  "meeting.live.listening": {
    "message": "Listening…",
    "description": "Meeting view: shimmering line after the transcript while recording. src/renderer/meeting.js (renderLive)"
  },
  "meeting.live.paused": {
    "message": "Paused. Nothing is recorded until you resume.",
    "description": "Meeting view: line after the transcript while paused. src/renderer/meeting.js (renderLive)"
  },
  "meeting.live.processing": {
    "message": "Rukoo Voice is finishing the transcript…",
    "description": "Meeting view: line after the transcript once the recording stopped and transcription goes on. src/renderer/meeting.js (renderLive)"
  },
  "meeting.live.nothing": {
    "message": "Nothing was transcribed in this meeting.",
    "description": "Meeting view: line in place of the transcript of a finished meeting without passages. src/renderer/meeting.js (renderLive)"
  },
  "meeting.track.mic": {
    "message": "Microphone",
    "description": "Meeting view transcript: the recorded track a gap or device mark is about, at the start of the mark line. src/renderer/meeting.js (markText)"
  },
  "meeting.track.call": {
    "message": "Call audio",
    "description": "Meeting view transcript: the computer audio track (the other people in the call), at the start of a gap or device mark line. src/renderer/meeting.js (markText)"
  },
  "meeting.track.screen": {
    "message": "Screen",
    "description": "Meeting view transcript: the screen track, at the start of a mark line. src/renderer/meeting.js (markText)"
  },
  "meeting.track.all": {
    "message": "Recording",
    "description": "Meeting view transcript: a mark about the whole recording, at the start of the mark line. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.user": {
    "message": "Marked",
    "description": "Meeting view transcript: a moment someone marked with Mark. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.userLabel": {
    "message": "Marked: {label}",
    "description": "Meeting view transcript: a marked moment with its label. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.dictation": {
    "message": "You dictated into another app for {length}",
    "description": "Meeting view transcript: a stretch you dictated into another app; length is like 18 sec. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.dictationOpen": {
    "message": "You dictated into another app",
    "description": "Meeting view transcript: a dictation stretch without a known end. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.gapCrash": {
    "message": "{track}: no audio for {length} while Rukoo Voice restarted",
    "description": "Meeting view transcript: a gap after the recorder crashed and restarted; track is Microphone or Call audio, length like 12 sec. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.gapDevice": {
    "message": "{track}: no audio for {length}, the device was gone",
    "description": "Meeting view transcript: a gap while the audio device was unplugged or failed; track is Microphone or Call audio. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.gapSuspend": {
    "message": "{track}: no audio for {length} while the computer slept",
    "description": "Meeting view transcript: a gap while the computer was asleep; track is Microphone or Call audio. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.gap": {
    "message": "{track}: no audio for {length}",
    "description": "Meeting view transcript: a gap without a known reason; track is Microphone or Call audio. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.unnamedDevice": {
    "message": "an unnamed device",
    "description": "Meeting view transcript: stands in for the device name in a device mark when Rukoo Voice gave none. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.deviceBack": {
    "message": "{track}: {device} is back",
    "description": "Meeting view transcript: an audio device came back; device is its name, such as Jabra Evolve2. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.deviceLost": {
    "message": "{track}: {device} was lost",
    "description": "Meeting view transcript: an audio device was lost. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.deviceMoved": {
    "message": "{track}: now from {device}",
    "description": "Meeting view transcript: the track moved to another device, such as a new default microphone. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.droppedDisk": {
    "message": "Screen recording stopped to keep recording audio: the disk is full",
    "description": "Meeting view transcript: Rukoo Voice dropped the screen recording because the disk filled up. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.droppedEncoder": {
    "message": "Screen recording stopped to keep recording audio: the video encoder failed",
    "description": "Meeting view transcript: Rukoo Voice dropped the screen recording because the encoder failed. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.droppedVideoDisk": {
    "message": "Video stopped to keep recording audio: the disk is almost full. Frames go on.",
    "description": "Meeting view transcript: Rukoo Voice stopped the screen video because the disk is almost full; the screen frames are still recorded. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.droppedVideoEncoder": {
    "message": "Video stopped: no video encoder on this computer worked. Frames go on.",
    "description": "Meeting view transcript: Rukoo Voice records the screen as frames only, because no video encoder worked; the screen frames are still recorded. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.droppedVideoDevice": {
    "message": "Video stopped: the graphics card couldn't start the video again",
    "description": "Meeting view transcript: Rukoo Voice stopped the screen video because the graphics device was reset and didn't come back with video support; the screen frames go on unless a mark that the screen recording stopped follows. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.droppedDevice": {
    "message": "Screen recording stopped to keep recording audio: the graphics card couldn't start again",
    "description": "Meeting view transcript: Rukoo Voice stopped recording the screen because the graphics device was reset and couldn't be started again. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.droppedCrash": {
    "message": "Screen recording stopped: the screen recorder failed too often",
    "description": "Meeting view transcript: Rukoo Voice's screen recorder stopped five times in a row, so Voice no longer records the screen in this meeting; the audio goes on. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.monitor": {
    "message": "Screen recording moved to {name}",
    "description": "Meeting view transcript: the user moved the screen recording to another monitor; name is the monitor's name from Windows. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.silent": {
    "message": "{track}: no sound from here while another app had its audio output open. If people were talking, macOS may not have let Rukoo Voice record it: check that Rukoo Voice is switched on under System Settings › Privacy & Security › Screen & System Audio Recording › System Audio Recording Only.",
    "description": "Meeting view transcript: Rukoo Voice recorded only exact silence on the call for a minute while another app had its audio output open. That can mean System Audio Recording is off for Voice on the Mac, or simply that nobody talked, so it is a hint, not an error; track is Call audio. Name the System Settings panes as macOS does in this language. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.silentUntil": {
    "message": "{track}: no sound for {length} while another app had its audio output open, then the call had sound",
    "description": "Meeting view transcript: Rukoo Voice warned that the call was silent, and the call had sound after {length}, so the warning was taken back; track is Call audio, length a duration such as 2 min. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.monitorLost": {
    "message": "The recorded monitor went away; screen recording moved to {name}",
    "description": "Meeting view transcript: the monitor Rukoo Voice recorded was unplugged or turned off, so it records another; name is that monitor's name. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.gapScreen": {
    "message": "Screen: not recorded for {length}",
    "description": "Meeting view transcript: a stretch without screen frames; length is a duration such as \"1 min, 5 sec\". src/renderer/meeting.js (markText)"
  },
  "meeting.mark.gapScreenCrash": {
    "message": "Screen: not recorded for {length} while the screen recorder restarted",
    "description": "Meeting view transcript: no screen frames while Rukoo Voice's screen recorder restarted; length is a duration. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.gapScreenLocked": {
    "message": "Screen: not recorded for {length} while the computer was locked",
    "description": "Meeting view transcript: no screen frames while the computer was locked or its session disconnected; length is a duration. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.gapScreenDevice": {
    "message": "Screen: not recorded for {length} while the graphics card restarted",
    "description": "Meeting view transcript: no screen frames while the graphics device was reset; length is a duration. src/renderer/meeting.js (markText)"
  },
  "meeting.mark.gapScreenMonitor": {
    "message": "Screen: not recorded for {length} while no monitor was connected",
    "description": "Meeting view transcript: no screen frames because Rukoo Voice had no monitor to record, for example while the only monitor was turned off or unplugged; length is a duration. src/renderer/meeting.js (markText)"
  },
  "meeting.sidebar.label": {
    "message": "Meetings",
    "description": "Sidebar in meeting mode: accessible name of the sidebar and of its list of meetings. src/renderer/app.js (renderShell), src/renderer/meeting.js (sidebarHtml)"
  },
  "meeting.sidebar.undated": {
    "message": "Not started yet",
    "description": "Sidebar in meeting mode: group heading of meetings Rukoo Voice has not reported a start time for yet. src/renderer/meeting.js (sidebarHtml)"
  },
  "meeting.sidebar.recordingTitle": {
    "message": "{title}, recording now",
    "description": "Sidebar in meeting mode: tooltip and accessible name of the meeting that records; title is the meeting title. src/renderer/meeting.js (sidebarHtml)"
  },
  "meeting.sidebar.itemTitle": {
    "message": "{title}, {time}",
    "description": "Sidebar in meeting mode: tooltip and accessible name of a meeting; time is when it started, such as 09:31. src/renderer/meeting.js (sidebarHtml)"
  },
  "meeting.sidebar.empty": {
    "message": "No meetings yet",
    "description": "Sidebar in meeting mode when Rukoo has no meetings. src/renderer/meeting.js (sidebarHtml)"
  },
  "meeting.indicator.short": {
    "message": "Recording",
    "description": "Title bar indicator while a meeting records but its start time is not known yet. src/renderer/meeting.js (renderIndicator)"
  },
  "meeting.indicator.recording": {
    "message": "Recording {title}. Open the meeting",
    "description": "Title bar indicator: tooltip and accessible name while a meeting records; a click opens it. title is the meeting title. src/renderer/meeting.js (renderIndicator)"
  },
  "meeting.indicator.paused": {
    "message": "{title} is paused. Open the meeting",
    "description": "Title bar indicator: tooltip and accessible name while a meeting is paused; a click opens it. src/renderer/meeting.js (renderIndicator)"
  },
  "calendar.event.record": {
    "message": "Record with Rukoo Voice",
    "description": "Event card: starts recording this event in Rukoo Voice; shown while Voice is connected, from 15 minutes before the event until it ends. src/renderer/calendar-event.js (popHtml)"
  },
  "agent.panel.placeholderMeeting": {
    "message": "Ask {name} about this meeting...",
    "description": "Chat panel input placeholder in a chat about a Rukoo Voice meeting; name is the agent name."
  },
  "agent.panel.meetingChip": {
    "message": "Meeting",
    "description": "Chat panel input: second line of the chip that shows the meeting a chat is about. src/renderer/agent/panel.js (contextChip)"
  },
  "agent.empty.titleMeeting": {
    "message": "What should {name} do with this meeting?",
    "description": "Chat panel empty state heading in meeting mode with a meeting on screen; name is the agent."
  },
  "agent.empty.bodyMeeting": {
    "message": "Reads what was said so far and writes what you ask for. Sending stays with you.",
    "description": "Chat panel empty state line under the heading with a meeting on screen."
  },
  "agent.empty.bodyMeetingOff": {
    "message": "Agents can't read meetings now. Turn it on in Settings > Meetings.",
    "description": "Chat panel empty state line with a meeting on screen while Settings > Meetings > Agents can read meetings is off."
  },
  "agent.meetingActions.catchup.label": {
    "message": "Catch me up",
    "description": "Chat panel quick action chip in a chat about a meeting: the agent sums up what was said since it last read the transcript. src/renderer/agent/actions.js (MEETING_ACTIONS)"
  },
  "agent.meetingActions.catchup.description": {
    "message": "What was said since you last asked",
    "description": "Chat panel slash command menu: one-line description of the Catch me up meeting action."
  },
  "agent.meetingActions.asked.label": {
    "message": "What did they ask me?",
    "description": "Chat panel quick action chip in a chat about a meeting: questions and requests for you in the transcript. src/renderer/agent/actions.js (MEETING_ACTIONS)"
  },
  "agent.meetingActions.asked.description": {
    "message": "Questions and requests for you",
    "description": "Chat panel slash command menu: one-line description of the What did they ask me? meeting action."
  },
  "agent.meetingActions.notes.label": {
    "message": "Write the notes",
    "description": "Chat panel quick action chip in a chat about a meeting: the agent writes the meeting notes with its own skill and stores them. src/renderer/agent/actions.js (MEETING_ACTIONS)"
  },
  "agent.meetingActions.notes.description": {
    "message": "With your meeting-notes skill, where you keep notes",
    "description": "Chat panel slash command menu: one-line description of the Write the notes meeting action."
  },
  "agent.meetingActions.followup.label": {
    "message": "Draft the follow-up email",
    "description": "Chat panel quick action chip in a chat about a meeting: the agent drafts the follow-up email in the composer. src/renderer/agent/actions.js (MEETING_ACTIONS)"
  },
  "agent.meetingActions.followup.description": {
    "message": "To everyone in the meeting, as a draft",
    "description": "Chat panel slash command menu: one-line description of the Draft the follow-up email meeting action."
  },
  "settings.groups.meetings": {
    "message": "Meetings",
    "description": "Settings overview: group heading above the Meetings row. src/renderer/settings.js"
  },
  "settings.meetings.title": {
    "message": "Meetings",
    "description": "Settings: title of the Meetings page and of its row on the overview. src/renderer/meeting-settings.js"
  },
  "settings.meetings.voice": {
    "message": "Rukoo Voice",
    "description": "Settings > Meetings: group heading above the status of Rukoo Voice. src/renderer/meeting-settings.js"
  },
  "settings.meetings.voiceHelp": {
    "message": "Rukoo Voice is a separate app that records meetings on this computer. Rukoo shows its meetings in the Meetings view and can't start Rukoo Voice for you.",
    "description": "Settings > Meetings: text under the Rukoo Voice status. src/renderer/meeting-settings.js"
  },
  "settings.meetings.statusAbsent": {
    "message": "Not running",
    "description": "Settings > Meetings: status pill, and the Meetings row on the overview, while Rukoo Voice is not running. src/renderer/meeting-settings.js"
  },
  "settings.meetings.statusAbsentHelp": {
    "message": "Start Rukoo Voice to record meetings.",
    "description": "Settings > Meetings: line under Rukoo Voice while it is not running. src/renderer/meeting-settings.js"
  },
  "settings.meetings.statusConnected": {
    "message": "Connected",
    "description": "Settings > Meetings: status pill while Rukoo is connected to Rukoo Voice. src/renderer/meeting-settings.js"
  },
  "settings.meetings.statusConnectedVersion": {
    "message": "Connected, version {version}",
    "description": "Settings > Meetings: status pill while connected; version is the version of Rukoo Voice, such as 0.1.0. src/renderer/meeting-settings.js"
  },
  "settings.meetings.statusConnectedHelp": {
    "message": "You can record a meeting from Rukoo or from Rukoo Voice.",
    "description": "Settings > Meetings: line under Rukoo Voice while connected and nothing records. src/renderer/meeting-settings.js"
  },
  "settings.meetings.statusRecording": {
    "message": "A meeting is recording now.",
    "description": "Settings > Meetings: line under Rukoo Voice while it records a meeting. src/renderer/meeting-settings.js"
  },
  "settings.meetings.statusConnecting": {
    "message": "Connecting",
    "description": "Settings > Meetings: status pill while Rukoo connects to Rukoo Voice. src/renderer/meeting-settings.js"
  },
  "settings.meetings.statusConnectingHelp": {
    "message": "Rukoo found Rukoo Voice and is connecting to it.",
    "description": "Settings > Meetings: line under Rukoo Voice while connecting. src/renderer/meeting-settings.js"
  },
  "settings.meetings.statusIncompatible": {
    "message": "Needs an update",
    "description": "Settings > Meetings: status pill when Rukoo and Rukoo Voice speak different bridge versions; the line under it says which to update. src/renderer/meeting-settings.js"
  },
  "settings.meetings.statusInsecure": {
    "message": "Not connected",
    "description": "Settings > Meetings: status pill when Rukoo refused the connection file of Rukoo Voice. src/renderer/meeting-settings.js"
  },
  "settings.meetings.privacy": {
    "message": "During a meeting",
    "description": "Settings > Meetings: group heading above hiding from screen recordings and agent access. src/renderer/meeting-settings.js"
  },
  "settings.meetings.hide": {
    "message": "Hide Rukoo from screen recordings during a meeting",
    "description": "Settings > Meetings: switch (meetingHideWindow) that hides every Rukoo window from screen capture while a meeting records. src/renderer/meeting-settings.js"
  },
  "settings.meetings.hideHelpWindows": {
    "message": "While a meeting records, screen recordings and screen shares in apps such as Teams and Zoom leave Rukoo's windows out. Remote access tools that capture the screen show them black; Remote Desktop still shows them.",
    "description": "Settings > Meetings on Windows: what hiding from screen recordings does and does not cover. src/renderer/meeting-settings.js"
  },
  "settings.meetings.hideHelpMac": {
    "message": "While a meeting records, Rukoo Voice and older screen recorders leave Rukoo's windows out. On macOS 15 and later, other apps' screen shares still show them.",
    "description": "Settings > Meetings on macOS: what hiding from screen recordings does and does not cover. src/renderer/meeting-settings.js"
  },
  "settings.meetings.hideHelpLinux": {
    "message": "Linux has no way to hide a window from screen recordings. Keep Rukoo on a monitor you don't share or record.",
    "description": "Settings > Meetings on Linux, in place of the switch: hiding is not possible there. src/renderer/meeting-settings.js"
  },
  "settings.meetings.notices": {
    "message": "While Rukoo Voice records the screen, Rukoo holds back mail and booking notifications so they stay out of the recording. Calendar reminders still show.",
    "description": "Settings > Meetings: line under the hide-from-capture switch about notifications held back while a meeting records the screen; the badge still counts. src/renderer/meeting-settings.js"
  },
  "settings.meetings.hiddenNow": {
    "message": "Rukoo is hidden from screen recordings now, because a meeting is recording.",
    "description": "Settings > Meetings: note under the switches while the windows are hidden. src/renderer/meeting-settings.js"
  },
  "settings.meetings.agents": {
    "message": "Agents can read meetings",
    "description": "Settings > Meetings: switch (meetingAgentsCanRead) that lets agents read transcripts and notes through the meeting tools. src/renderer/meeting-settings.js"
  },
  "settings.meetings.agentsHelp": {
    "message": "Your agents read transcripts and your meeting notes through Rukoo's meeting tools, marked as text from other people. Turned off, they get nothing from your meetings.",
    "description": "Settings > Meetings: what the Agents can read meetings switch does. src/renderer/meeting-settings.js"
  },
  "settings.meetings.connection": {
    "message": "Connection",
    "description": "Settings > Meetings: title of the row with the connection status of Rukoo Voice; the status pill is on its right. src/renderer/meeting-settings.js"
  },
  "settings.agents.removedAgent": {
    "message": "Removed agent",
    "description": "Name shown for a chat whose custom agent the user removed in Settings > Agents. src/renderer/settings.js, src/renderer/agent/panel.js"
  },
  "settings.agents.preset.booper": {
    "message": "Booper",
    "description": "Settings > Agents: product name of a custom agent on Booper, beside its name and in the agent menus. A product name; keep it."
  },
  "settings.agents.preset.runs": {
    "message": "Runs API agent",
    "description": "Settings > Agents: kind of a custom agent on a server with the Hermes Runs API, beside its name and in the agent menus."
  },
  "settings.agents.preset.openai": {
    "message": "OpenAI-compatible agent",
    "description": "Settings > Agents: kind of a custom agent on a server with an OpenAI-compatible Chat Completions endpoint, beside its name and in the agent menus."
  },
  "settings.agents.presetHint.booper": {
    "message": "An agent on Booper, reached through its app channel.",
    "description": "Settings > Agents > Add an agent: hint under the Booper choice."
  },
  "settings.agents.presetHint.runs": {
    "message": "Any server with the Runs API of Hermes Agent.",
    "description": "Settings > Agents > Add an agent: hint under the Runs API choice."
  },
  "settings.agents.presetHint.openai": {
    "message": "Any agent framework or model server with an OpenAI-compatible Chat Completions endpoint.",
    "description": "Settings > Agents > Add an agent: hint under the OpenAI-compatible choice."
  },
  "settings.agents.customAdd": {
    "message": "Add an agent",
    "description": "Settings > Agents: row, and title of the dialog, that adds a custom agent. src/renderer/settings.js"
  },
  "settings.agents.customAddHelp": {
    "message": "Booper, another Runs API server, or an OpenAI-compatible agent",
    "description": "Settings > Agents: description of the Add an agent row."
  },
  "settings.agents.customRemote": {
    "message": "Let {name} use Rukoo from another computer",
    "description": "Settings > Agents, custom agent card: toggle that lets the agent call Rukoo's tools over Tailscale. {name} is the agent's name."
  },
  "settings.agents.customRemoteHelp": {
    "message": "For an agent elsewhere in your tailnet. Rukoo then also listens on its Tailscale address.",
    "description": "Settings > Agents, custom agent card: description of the remote toggle while it is off."
  },
  "settings.agents.customHistory": {
    "message": "Send earlier messages",
    "description": "Settings > Agents, OpenAI-compatible agent card: toggle that sends the chat's earlier turns with each message."
  },
  "settings.agents.customHistoryHelp": {
    "message": "For a server that doesn't keep the conversation itself. Turn it off for one that does.",
    "description": "Settings > Agents, OpenAI-compatible agent card: description of the earlier messages toggle."
  },
  "settings.agents.customCopy": {
    "message": "Copy {name} setup",
    "description": "Settings > Agents, custom agent card: row that copies what lets the agent use Rukoo's tools. {name} is the agent's name."
  },
  "settings.agents.copyBooperHelp": {
    "message": "The block for booper.yaml and the variable with Rukoo's token. It holds a token, so don't share it.",
    "description": "Settings > Agents, Booper card: description of the copy setup row."
  },
  "settings.agents.copyCustomHelp": {
    "message": "Rukoo's MCP server, as most agents take it. It holds a token, so don't share it.",
    "description": "Settings > Agents, custom agent card (not Booper): description of the copy setup row."
  },
  "settings.agents.copiedCustom": {
    "message": "Copied. Add it to {name} on this computer, then restart {name}.",
    "description": "Settings > Agents: toast after copying a custom agent's setup for an agent on this computer."
  },
  "settings.agents.copiedCustomRemote": {
    "message": "Copied, with {url}. Add it to {name} on its computer, then restart {name}.",
    "description": "Settings > Agents: toast after copying a custom agent's setup for an agent on another computer. {url} is Rukoo's Tailscale address."
  },
  "settings.agents.customRemove": {
    "message": "Remove {name}",
    "description": "Settings > Agents, custom agent card: row, and title of the confirmation, that removes the agent."
  },
  "settings.agents.customRemoveConfirm": {
    "message": "Rukoo forgets {name}'s address and key, and the token that gives {name} Rukoo's tools stops working. Your chats with {name} stay, but they can't take new messages.",
    "description": "Settings > Agents: confirmation before removing a custom agent."
  },
  "settings.agents.customRemoveOk": {
    "message": "Remove",
    "description": "Settings > Agents: confirm button that removes a custom agent."
  },
  "settings.agents.customRemoved": {
    "message": "{name} removed",
    "description": "Settings > Agents: toast after removing a custom agent."
  }
};
  if (typeof module === 'object' && module.exports) module.exports = messages;
  else (root.RukooLocales ||= {})['en'] = messages;
})(globalThis);
