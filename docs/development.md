# Developing Rukoo

## Run from source

Rukoo needs Node.js 22 or newer and a graphical desktop.

```bash
npm install
npm start
```

`npm start` goes through `scripts/start.js`, which clears `ELECTRON_RUN_AS_NODE`. Terminals that run on Electron, such as VS Code and T3 Code, export that variable, and it makes `electron.exe` behave like plain Node.

`SEM_DATA_DIR` points Rukoo at a separate profile, so a test run doesn't touch your own accounts and settings.

## Build packages

```bash
npm run dist
```

This builds the packages for the current platform in `dist/`: the Windows installer and portable executable, the macOS DMG and ZIP, or the Linux x64 AppImage and `.deb`. `npm run dist:win`, `npm run dist:mac` and `npm run dist:linux` pick a platform explicitly.

On a Mac, use `npm run dist:mac`. It builds both architectures and signs the app ad-hoc, as CI does. Plain `npm run dist` looks for a Developer ID Application certificate in your keychain and, without one, leaves the app unsigned. macOS doesn't deliver notifications from an unsigned app, so test notifications with an ad-hoc or Developer ID signed build.

## Tests

```bash
npm test          # unit tests for the engine and mail parsing, plus a live IMAP test
npm run test:e2e  # Playwright drives the real Electron app; chat tests use scripted agents (SEM_AGENT_FAKE=1)
```

The live IMAP tests create a throwaway mailbox on [Ethereal](https://ethereal.email) and skip themselves when it's unreachable.

In the Electron tests (`SEM_HIDDEN=1`) Send goes out at once, so the suites don't wait out the Undo send notice. `SEM_UNDO_SEND=1` turns the wait back on; `test/e2e/undo-send.spec.js` does that.

Headless Electron tests on Linux need an X display and a window manager, so minimize and restore events work. The CI workflows show the Xvfb and Openbox setup with a 1920×1200 display. CI tests and builds on Ubuntu 22.04, Windows and macOS. On macOS it runs the unit tests and only the Mac-specific Electron tests in `test/e2e/macos.spec.js`, on both Intel and Apple Silicon, from source and again against the built app. On Windows and Ubuntu it splits the Electron tests over three runners with `--shard=N/3 --fully-parallel`, so a run takes about a third of the time and a failed shard can be rerun on its own. When you run the whole Electron suite on a Mac yourself, the cases written for Windows or Linux behavior skip themselves, and `macos.spec.js` checks the Mac side. The demo suite doesn't verify real mail synchronization, native tray availability or live agent authentication on every Linux desktop.

## Screenshots

`npm run screenshots` takes the documentation screenshots again in `docs/screenshots/`, in light and dark mode. The capture script always uses English with a fresh demo profile. For screenshots you don't mean to commit, run `node scripts/screenshots.js tmp/screenshots`.

## Code map

The renderer runs sandboxed with context isolation. It can only call the methods in the IPC table in `main.js`, through the bridge in `preload.js`.

| Path | Role |
| --- | --- |
| `src/main/main.js` | Main and compose windows, IPC table, notifications, taskbar badge, unsubscribe |
| `src/main/engine.js` | Accounts, local cache, sync, flags, moves, sending, drafts, VIPs and spam |
| `src/main/outbox.js` | Undo send: holds an email after Send until its notice runs out, a quit or Undo, and a refused one until the main window has opened it |
| `src/main/imap.js` | IMAP via [imapflow](https://github.com/postalsys/imapflow), SMTP via nodemailer |
| `src/main/demo.js` | The demo account: same interface as `imap.js`, backed by a local JSON file |
| `src/main/google.js` | Google sign-in: loopback redirect, PKCE, token refresh |
| `src/main/calendar/` | The calendar: cache, sync and changes for every account (`index.js`) and checks on what a window sends (`ipc.js`); Google Calendar (`google.js`), CalDAV (`caldav.js`) and the demo calendar (`demo.js`); iCalendar objects (`ics.js`) and their time zone definitions (`vtimezone.js`); clock times in named zones (`zone.js`); open time (`availability.js`, and `free.js` across your calendars); reminders (`reminders.js`, shown by `reminder-runner.js`) |
| `src/main/booking/` | The booking page on this computer: publishing open times, taking bookings into the calendar and keeping the two in step (`index.js`), settings and guest checks (`rules.js`), and the key that opens guests' details (`keys.js`) |
| `src/main/categories/` | Inbox categories: which category an email is in and storing it (`index.js`), the rules (`rules.js`), the header signals a sync collects (`signals.js`) and Jev (`jev.js`) |
| `src/main/account/` | The Rukoo account: keys and encryption (`crypto.js`), the server's API (`client.js`) and the records kept here and synced (`index.js`) |
| `src/main/agents/` | The chat panel's main side: conversations and approvals (`hub.js`), Rukoo's MCP server and tools (`mcp.js`, `tools.js`), skills (`skills.js`, and the agents' own in `ownskills.js`), files attached to messages (`chatfiles.js`), the Hermes, Claude Code, Codex and OpenClaw adapters with the HTTP helpers Hermes and OpenClaw share (`http.js`), and pairing and turns for Rukoo on a phone (`phonehost.js`) |
| `src/main/windowstate.js` | Remembers window size and position |
| `src/main/tray.js` | The tray icon, its tooltip and menu; `main.js` decides when there is one |
| `src/main/zoom.js` | Zoom steps and the zoom keys; `main.js` applies the level to every window |
| `src/main/logos.js` | Fetches and caches sender logos |
| `src/main/pdftext.js` | Reads the text of a PDF for agents that can't open one themselves |
| `src/main/pdfread.js`, `pdfchild.js` | Run `pdftext.js` in a process of its own, with a time limit and a memory cap |
| `src/renderer/` | Plain HTML, CSS and ES modules, no framework or bundler |
| `src/renderer/app.js` | The main window: sidebar, list, reading pane, selection, undo |
| `src/renderer/composer.js` | The editor, inline or in a window, with autosave, the calendar button and the booking link under the signature |
| `src/renderer/compose.js` | The compose window around the editor |
| `src/renderer/mentions.js`, `mention-match.js` | @-mentions in the editor: the suggestion list, the mention and the Cc it adds; which text is a mention and who fits it |
| `src/renderer/calendar.js` | The month, week, day and upcoming views, the grid and its drags, the sidebar and the keys |
| `src/renderer/calendar-shared.js`, `calendar-event.js` | State and date helpers the calendar views share; the event card, the quick popover and the event dialog |
| `src/renderer/calendar-free.js`, `calendar-settings.js`, `booking-settings.js` | The availability panel, Settings → Calendar and Settings → Booking page |
| `src/renderer/categories-settings.js`, `rukoo-account.js` | Settings for inbox categories and the Rukoo account |
| `src/renderer/agent/` | The chat panel (`panel.js`), its quick actions, and the components ported from Beautiful UI (`bui.js`, `agent.css`) |
| `src/i18n/` | The English source catalog, the Dutch translations and the locale registry. See [the translation guide](translations.md) |
| `skills/` | Rukoo's skills in the Agent Skills format, shipped with the app |
| `server/` | The account and booking server, a Cloudflare Worker, and its protocol (`PROTOCOL.md`). In `server/src/`: routes (`index.js`), a Durable Object per account (`account.js`) and per booking page (`booking.js`), the owner API for booking pages (`owner.js`), the pages people open (`web.js`, `pages.js`, `booking-text.js`), the JSON API (`api.js`), the MCP endpoint (`mcp.js`), what assistants read (`agent-text.js`), open times (`slots.js`) and the sealing of guests' details (`seal.js`) |
| `integrations/hermes/` | The stdio bridge that lets a Hermes agent on another machine reach Rukoo's tools over Tailscale, on computers and Android phones |
| `android/` | Rukoo for Android, a native Kotlin and Jetpack Compose app. See [Rukoo for Android](android.md#build-and-develop) |

## Releases

Versions follow [Semantic Versioning](https://semver.org): a patch release fixes things, a minor release adds features, and a major release breaks compatibility of behavior, integrations or stored data. Previews use `-alpha.N`, `-beta.N` or `-rc.N` and publish as prereleases.

A release is worth making when a new download gives users something: a feature, a visible fix, a security fix or a dependency change that affects the app. Documentation, tests and tooling wait for the next one.

Releases start manually after a version tag exists. Pushing a branch or tag doesn't start a build. To prepare one:

1. Run `npm version <version> --no-git-tag-version`, commit the package and lockfile changes with the README's line about the latest release, and merge the reviewed PR into `main`. Before merging, run `npm test`, `npm run test:e2e` and `npm run dist`, and start the built app with a demo profile.
2. Write the release as a draft on GitHub. Title it `v<version> - Added …` when it adds features, or `v<version> - Fixed …` when it only fixes things, for example `v1.1.0 - Added CalDAV calendars and Microsoft sign-in`. The notes tell users what changed, with known limitations and migration steps. `gh release create v1.1.0 --draft --title "…" --notes-file notes.md` writes the draft without creating the tag.
3. On the tested `main` commit, create and push an annotated tag, for example `git tag -a v1.1.0 -m "Rukoo Mail 1.1.0"` followed by `git push origin v1.1.0`.
4. Build the Android APK from the tag: in `android/`, run `./gradlew assembleRelease` with `signing.properties` in place, and upload `app/build/outputs/apk/release/app-release.apk` to the draft as `Rukoo-Android-<versionName>.apk`, for example with `gh release upload v1.1.0 Rukoo-Android-0.1.0.apk`. Add its SHA-256 to the notes. The workflow doesn't build the APK because the signing key stays with the maintainer, and Android only installs an update signed with the same key. When `android/` changed since the last release, raise `versionCode` and `versionName` in `android/app/build.gradle.kts` first.
5. Open **Actions → Release desktop packages → Run workflow**, select `main`, and enter `v1.1.0`. Or run `gh workflow run release.yml --ref main -f tag=v1.1.0`.
6. The workflow checks that the tag matches the package and lockfile, runs unit and Electron tests, builds the Windows, macOS and Linux packages and verifies their checksums. It then uploads all eight packages to the draft and publishes it.

Tags such as `v1.1.0-rc.1`, and drafts marked as a pre-release on GitHub, publish as prereleases without becoming the latest stable release. A stable release for an older version line does become the latest, so clear that on GitHub afterwards.

The workflow stops when the tag has no draft, when the draft's title doesn't start with the tag, or when the release is already published. Never move or reuse a published tag; ship a new version for corrections. If publication fails, fix the cause and choose **Re-run failed jobs**: the build is kept, and the upload replaces files from the earlier attempt. Publication uses GitHub Actions' built-in token and needs no personal token or extra secrets.

The app's update check reads the repository from `rukoo.releases` in `package.json`, so a fork points it at its own releases.

### Nightly builds

`.github/workflows/nightly.yml` runs every night at 03:17 UTC. It takes the newest `main` commit whose **Test and package** run passed on push, so a red `main` keeps the nightlies on the last green commit. GitHub's built-in token may not tag a commit whose workflow files differ from `main`'s, so when `.github/workflows/` changed since that commit there is no nightly until `main` is green again. When that commit already has a nightly, or is the latest stable release, nothing is built. Otherwise it builds the same eight packages as a release, without running the suites again, and publishes them as a pre-release for the Nightly update channel, with `latest.yml` and `latest-linux.yml`.

The version is the next patch after the package version followed by `-nightly.<date>.<run>`, for example `v2.3.1-nightly.20261109.42`, so the next stable release is newer than the nightlies before it. When `package.json` already has a version that isn't released yet, the nightlies use that version instead of the next patch. The workflow keeps the 14 newest nightlies and deletes older ones with their tags, so a stable release stays within the 100 releases the update check reads. **Actions → Nightly desktop packages → Run workflow** starts one by hand; `force` builds even when the commit already has a nightly. Nightly Mac packages are ad-hoc signed.

### Signed Mac builds

CI makes ad-hoc signed Mac packages. These are **not Developer ID signed or notarized**, and Gatekeeper may block downloaded builds. Signed public releases need the maintainer's Apple credentials, which this repository doesn't contain.

For a signed release, configure the repository secrets `MAC_CSC_LINK` (the Developer ID Application certificate as a base64 P12), `MAC_CSC_KEY_PASSWORD`, `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD` and `APPLE_TEAM_ID`, then select `sign_macos` when starting the release workflow. That option needs all of these credentials and verifies the resulting signature and stapled ticket. Without it, the workflow labels the Mac builds as ad-hoc signed only; don't describe those as notarized releases.

Live provider sign-in, notification delivery, and Developer ID and Keychain continuity across upgrades still need checking with a signed release on a user's Mac.
