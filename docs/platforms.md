# Platforms

| Platform | Status | Packages |
| --- | --- | --- |
| Windows 10 and 11, x64 | Supported | Installer and portable `.exe` |
| Linux, x64 | Supported | `.deb` for Ubuntu and Debian, AppImage for other distributions |
| macOS 13 or newer, Apple Silicon and Intel | Experimental | DMG and ZIP, notarized only when the release notes say so |
| Android 12 or newer, built for the Galaxy Z Fold7 | Experimental | APK. See [Rukoo for Android](android.md) |

Download them from [GitHub Releases](https://github.com/MajesteitBart/rukoo/releases). Each release has `SHA256SUMS.txt` for the desktop packages, and its notes give the APK's checksum. You can also [run Rukoo from source](development.md#run-from-source) on Windows, macOS and Linux.

## Windows

The installer and the portable `.exe` aren't signed yet, so SmartScreen warns before the first start: choose "More info", then "Run anyway".

Closing the window keeps Rukoo running in the tray, so mail keeps syncing and new mail still notifies you. Clicking the tray icon or a notification brings the window back where it was, with a half-written reply still in it. The icon's menu opens Rukoo, starts a new message, syncs or quits. Its tooltip counts unread mail, and a dot on the icon repeats the taskbar badge. Minimizing can hide the window in the tray as well. Turn both off in Settings → General to have closing quit Rukoo again. Windows 11 first puts the icon among the hidden icons, behind the arrow on the taskbar; drag it onto the taskbar to keep it in view.

For [email links](mail.md#email-links), the installer offers Rukoo Mail to Windows as an email app, and the installed app does so again when it starts if needed. The portable build does so when you click Use Rukoo Mail under Settings → General → Email links. That button opens the Windows Default apps page, where you pick Rukoo Mail, because Windows doesn't let an app make itself the default. The row shows which app opens the links now.

The registration is for your Windows user only. Uninstalling removes it when it belongs to that installation. A portable copy keeps its own until you pick another app, and moving the portable `.exe` updates it at the next start.

The profile is `%APPDATA%\Rukoo Mail`. `data`, `logos`, `skills` and `google-oauth.json` live under it. Credentials use Windows DPAPI.

## macOS (experimental)

Few people have used Rukoo on a real Mac yet, and builds are only notarized when the release notes say so. Native CI runs the unit tests and the Mac-specific Electron tests (`test/e2e/macos.spec.js`) on Apple Silicon and Intel; the rest of the Electron suite runs on Windows and Linux only. If you try it, please report your findings in [issue #74](https://github.com/MajesteitBart/rukoo/issues/74): what works, what breaks, your Mac model and your macOS version.

The bundled Electron version needs macOS 13 or newer. Choose `arm64` for Apple Silicon or `x64` for Intel. Open the DMG, drag Rukoo Mail to Applications and launch it there; ZIP builds contain the same app bundle.

Release builds are ad-hoc signed unless the release notes confirm Developer ID signing and notarization. Gatekeeper may block an ad-hoc signed download.

Notifications need Rukoo Mail to be allowed under **System Settings → Notifications**. An ad-hoc signed build may not show them at all, and macOS doesn't tell Rukoo when it drops one. **Settings → General → Send a test notification** shows whether one reaches you. Rukoo never shows a desktop notification for new mail while its window is in front; the window shows the mail itself then (see [Notifications](mail.md#notifications-and-settings)). Sound for new mail, in the same place, plays whether or not notifications arrive.

The red close button hides the main window while mail keeps syncing. Click Rukoo in the Dock to return to the same reader or draft. **Rukoo Mail → Quit Rukoo Mail** or **⌘Q** quits, asking about unsaved drafts. A separate compose window still asks about its unsaved draft when closed. Tray behavior is off by default; unread counts can appear on the Dock icon.

Use Command instead of Ctrl for mail, compose, calendar and zoom shortcuts. The mouse wheel doesn't zoom on a Mac, with or without Command, because Chromium only zooms with Ctrl and the wheel on other systems. Mark as read is **⌘ShiftU**, mark as unread is **⌘U**, and **⌘Q** stays Quit. **⌘Space** stays Spotlight, so in Rukoo's own scheme the key that selects one email at a time is **X** or **⌃Space**; see [Selecting with the keyboard](mail.md#selecting-with-the-keyboard). The native Edit menu supports copy, paste, undo and selection, and shortcuts in the interface show ⌘. Zoom and traffic-light spacing apply to both main and compose windows.

For email links, choose **Settings → General → Email links → Use Rukoo Mail** after installing the app. This works only for a copy in Applications or `~/Applications`. When Rukoo runs from the DMG or from Downloads, Settings asks you to move it there first, so email links never point at a copy that disappears later. macOS may ask for confirmation. If needed, choose Rukoo Mail as the default email reader in **Apple Mail → Settings → General**. Links delivered while Rukoo starts wait until its windows are ready.

macOS 15 and later may ask whether Rukoo Mail can find devices on your local network, sometimes right after launch. Rukoo doesn't need that access with default settings, so denying it is safe. It may matter for a mail or calendar server on your own network, and for remote agent access, which connects Rukoo with an agent or a paired phone on another device over Tailscale. If one of those stops working after you denied it, turn Rukoo Mail on under **System Settings → Privacy & Security → Local Network**.

The profile is `~/Library/Application Support/Rukoo Mail`; `data`, `logos`, `skills` and `google-oauth.json` live under that directory. Credentials use Keychain; unlock the login keychain if Rukoo can't save them. Agents started from Finder also search `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, and the folders of the Node version managers Volta, fnm and nvm; see [agents](agents.md).

To build both architectures on a Mac, run `npm run dist:mac`. It signs the app ad-hoc, as CI does, so it needs no Apple certificate. An unsigned app can't post notifications; [Build packages](development.md#build-packages) explains when a build ends up unsigned.

## Linux

Linux builds target x64 desktops. The `.deb` is for Ubuntu and Debian; the AppImage is a portable option for other distributions.

```bash
# Install the Debian package:
sudo apt install ./Rukoo-Mail-2.1.2-amd64.deb
# Or run the portable build:
chmod +x Rukoo-Mail-2.1.2-x86_64.AppImage
./Rukoo-Mail-2.1.2-x86_64.AppImage
```

Use the version in your downloaded filename. AppImages need FUSE 2; where FUSE is unavailable, `--appimage-extract-and-run` runs without mounting. Run Rukoo as your desktop user and keep Chromium's sandbox on; the Debian package includes Electron's sandbox setup. Some distributions restrict unprivileged user namespaces, so prefer the installed package when a portable build can't start under the system's sandbox policy.

The Debian package installs a launcher and advertises `mailto:` support. Choose **Settings → General → Email links → Use Rukoo Mail**, or select it in your desktop's default applications. This uses `xdg-mime` from `xdg-utils`. An AppImage alone doesn't install a desktop entry or register its temporary mount as an email handler.

Close-to-tray and minimize-to-tray start off on Linux. Turn them on only on a desktop with a visible status tray; GNOME may need an AppIndicator extension. Notifications depend on the desktop notification service, and Linux has no taskbar count badge.

Real accounts and agent keys need an unlocked GNOME Keyring (Secret Service) or KWallet. Install and unlock one in your desktop session if Rukoo reports that it can't save credentials. Rukoo doesn't fall back to Electron's insecure `basic_text` storage on Linux.

The profile is `$XDG_CONFIG_HOME/Rukoo Mail`, or `~/.config/Rukoo Mail` when that's unset; `data`, `logos`, `skills` and `google-oauth.json` live under it. Don't copy encrypted credentials between operating systems; sign in again on the new system.

To build the packages on Linux, run `npm run dist:linux`.
