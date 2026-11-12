# Meetings

[Rukoo Voice](../voice/README.md) records your meetings on this computer: your microphone, the call audio and, if you want, the screen. Rukoo shows them as they happen, next to your agent. **Meet** in the switch at the top of the sidebar, or `Ctrl+3`, replaces the message list and the reader with the meeting view.

Rukoo can't start Rukoo Voice. While Voice isn't running, the view says so and still shows the meetings Rukoo kept from before.

## Recording

There are three ways to start a recording:

- **Record a meeting** at the top of the sidebar in meeting mode.
- **Record with Rukoo Voice** on a calendar event's card, from 15 minutes before the event until it ends, while Voice is connected and nothing records yet. The meeting takes the event's title and is linked to it.
- Voice's own pill or tray. Rukoo opens the meeting as soon as Voice tells it, and links the calendar event that overlaps its start when exactly one fits.

### The screen

Voice can record the screen with the audio. Its Settings (Meetings, Screen recording) set the default: off, frames or video. Frames are still images, saved when the screen changes, with the text Windows reads on them on this computer. Video adds a video of the screen, made by the computer's hardware video encoder; without one, Voice records frames only.

Record in the sidebar, Record in the empty meeting view and Record with Rukoo Voice on a calendar card use Voice's default. The arrow beside each of them chooses for this meeting: Voice's default (the menu says which), Audio only, Audio and screen frames, or Audio and screen video. A meeting started from Voice's pill or tray uses Voice's default.

Voice records one monitor: the one with the window you're working in, which skips Rukoo while Rukoo is hidden from screen recordings. In a recording meeting, **Screen** in the header lists the monitors and moves the recording to another one; Voice's pill has a button for the next monitor too. A move shows in the transcript.

When Voice starts a meeting itself, it waits up to half a second for Rukoo to hide its windows before it captures the first frame. Windows draws a coloured border around the recorded monitor while Voice captures it.

While a meeting records, a red dot and the meeting's clock sit in the title bar in every mode. A click opens the meeting. Its header has Mark, Pause or Resume, and Stop, and Screen when the meeting records the screen. Stop asks once more ("Stop recording?") before it stops, as Voice's pill does, because a meeting can't be recorded again.

Voice keeps recording when Rukoo quits or loses the connection. When Rukoo comes back, it asks Voice for everything it missed.

## The meeting view

Use the edit button beside the title, or **Rename meeting** in the sidebar's right-click menu, to rename a meeting. Enter saves, Escape cancels; titles have 1 to 200 characters. Rukoo keeps the title across restarts and uses it in meeting search and transcript tools. Voice and the bucket retain their original title.

The header menu and sidebar offer **Delete meeting** once recording and processing have finished. The confirmation names the data being removed: audio, screen recordings, transcript, notes, speaker names and corrections. Deletion is permanent on this computer. Voice queues a bucket index tombstone, including while offline, so the meeting cannot be restored after the next connection; encrypted objects remain subject to the bucket's lifecycle rules. Chats stay and show **Meeting deleted**. An older Voice needs an update before it can delete a meeting.

While recording or paused, **Screen** in the header and the pill's screen button offer **Off**, **Frames**, **Video** and a monitor. These choices affect the current meeting. Off stops screen capture while audio continues; selecting Frames or Video starts it again, including in a meeting that began with audio only. The transcript marks each change. Before capture starts, every connected Rukoo must confirm its windows are protected. A failed confirmation leaves the previous mode unchanged. Live mode controls require the updated Voice; older Voice keeps the existing recording behavior.

The transcript fills the view. Each line has its meeting time in the margin, and the voice's name shows once per stretch. Lines you dictated into another app during the meeting are greyed out and tagged Dictation: the call heard them, but they weren't part of the conversation. Gaps in the audio, a device that came back and your marks sit between the lines.

In a meeting that records the screen, each frame sits at the moment the screen changed: a thumbnail with the first line of the text on it. A click on the thumbnail, or Enter on the row, opens it at the transcript's width with all of its text, which you can select and copy; **Text on screen** does the same. Rukoo asks Voice for an image when its row comes into view and keeps the last 60 of the meeting on screen. Marks say when Voice stopped the video or the whole screen recording to keep the audio, when it moved to another monitor, and when there were no frames for a while, for example while the computer was locked.

Screen frames with nearly unchanged text fold into one transcript row, even when people spoke between them. The +count button opens the other frames in a strip. A new slide or document gets its own row at its time. The Screen toggle above the transcript hides all frames and remembers your choice on this device. This only changes the view: the meeting tools can still read every saved frame.

The header says what the meeting records of the screen (Screen: frames or Screen: video). While it records with Rukoo not hidden from screen recordings, **Visible in screen recordings** warns that Rukoo can show in the frames.

Double-click a line, or press Enter on it, to correct its text; Enter saves and Esc cancels. The right-click menu brings back what Voice recognized. In the transcript the arrow keys move between lines.

Beside the transcript:

- **Calendar event**: the event the meeting is linked to. Change picks another event within 15 minutes of the start, or unlinks it, and a click on the title opens the event in the calendar.
- **People**: the voices Voice heard. Your microphone is You. With speaker labels on in Rukoo Voice, each voice on the call gets its own row, Speaker 1, Speaker 2 and so on; without them the whole call is one voice, Speaker 1. Pick who each one is from the linked event's guests, or type a name. A guest you already picked for another voice is marked. Rukoo fills in a name by itself only when the call has one voice and the event exactly one other guest; when a second voice turns up later, Rukoo asks you to check that name. A name stays on its voice for the whole meeting, also when Voice settles later who said a passage.
- **Your notes**: use the composer's formatting toolbar and shortcuts for headings, lists, bold text and links. Notes save as Markdown a moment after you stop typing and when you leave the editor. The wider side editor keeps the transcript visible; the Notes tab gives you the full width. Your agent reads the saved Markdown.

The Transcript, Notes and Details tabs are available at every width. In a narrow window each tab takes the whole view.

Notes have a 100,000-character limit on saved Markdown, including its formatting and escaping. If a save exceeds it, Rukoo keeps the last saved copy and shows "Not saved: shorten the notes". The unsaved text stays in the editor while the app is open, including when you switch meetings. Shorten it before closing the app to save your changes.

The sidebar lists the meetings by day, newest first. With the chat open the sidebar becomes a rail, where each meeting shows its day of the month.

## The agent

With the chat open in meeting mode, the chat is about the meeting on screen: the panel opens that meeting's newest chat, or starts a new one about it. Its quick actions are Catch me up, What did they ask me?, Write the notes and Draft the follow-up email.

In meeting chats, `get_context` returns meeting metadata and points to `read_meeting_transcript`; an email left open in Mail does not appear as the open message.

The agent reads the meeting through [Rukoo's meeting tools](agents.md#rukoos-tools). Each turn says how many lines are new since the agent last read the transcript; the transcript itself never goes into the turn. Everyone in a meeting is a third party, so the tools hand the transcript over as untrusted text. In a meeting that records the screen, the agent can look at the frames at a moment or over a stretch; anything can be on a screen, so their text is untrusted too, and a video clip's file path goes only to an agent on this computer. No tool starts or stops a recording.

The agent can also get a link to the meeting's audio, one frame or up to five minutes of its video, which works for five minutes and only from where it asked. Agents on another computer, such as Clark, get video this way. Rukoo Voice serves the media, so it must be running; with bucket sync on, Voice fetches media back from your bucket when this computer no longer has it.

## Settings

Settings → Meetings shows whether Rukoo Voice is connected, and has two switches, both on by default:

- **Hide Rukoo from screen recordings during a meeting.** While a meeting records, every Rukoo window is left out of screen capture.
  - Windows 10 2004 and later: screen recordings and screen shares in apps such as Teams and Zoom leave Rukoo out. Remote access tools that capture the screen show Rukoo's windows black; Remote Desktop still shows them.
  - macOS: Rukoo Voice and older screen recorders leave Rukoo out. On macOS 15 and later, other apps' screen shares still show it.
  - Linux has no way to hide a window. Keep Rukoo on a monitor you don't share or record.
- **Agents can read meetings.** Off, the meeting tools give agents nothing from your meetings, and a new chat about a meeting is named after your first message instead of the meeting.

While a meeting records the screen, Rukoo holds back its mail and booking notifications, so their text stays out of the frames. It also holds them back while the mode is uncertain, including during reconnection or a switch. The badge still counts new mail. Off allows notifications again; enabling capture holds them back before the first frame. Notifications held earlier are not shown later. Calendar reminders still show, because a missed reminder costs more than its title on the screen.

In meeting mode, the status line at the bottom of the sidebar says how Voice is, and opens these settings.

## Limits

- Rukoo shows the meetings of the Rukoo Voice on this computer. The phone has no meetings.
- Speaker labels tell voices apart, not people. Voice can hear one person as two voices or two similar voices as one, and it doesn't label people who share your microphone.
- Rukoo keeps a copy of each transcript in its profile, so the view and the tools work while Voice is closed. Voice's automatic media retention leaves transcripts and notes intact. An explicit meeting deletion also removes your notes and other owned data.
- Frame images stay with Voice. Rukoo keeps their text, so while Voice isn't running a frame's row shows its text, and an image only when Rukoo loaded it before. After Voice removed the meeting's screen media with its audio, the rows keep their text and say the image is gone.
- Rukoo doesn't play a meeting's video. Agents can get a clip or a link to it (see [the meeting tools](agents.md#rukoos-tools)).
- Windows' capture border stays around the recorded monitor; Voice doesn't hide it.
- Other apps' notifications and pop-ups on the recorded monitor end up in the frames. Rukoo holds back only its own.
- With Rukoo's hide switch off, a meeting started from Rukoo records Rukoo's monitor, because Rukoo is the window you're working in. Move it with Screen in the header or with the pill's monitor button.
- When Windows doesn't hide a Rukoo window, the meeting's header says "Visible in screen recordings" and Rukoo tries again every second. Until a try works, Rukoo can show in the frames: a meeting Voice starts gets no confirmation from Rukoo and starts its frames after half a second.
- A Rukoo that isn't connected to Voice when a meeting starts from Voice can show in that meeting's first frames. Voice waits up to 5 seconds only for a Rukoo it has seen since Voice started, so right after Voice restarts it doesn't wait at all.
- Native file dialogs and system notifications can't be hidden from screen capture.
- Media links need Rukoo Voice running on this computer.
