<p align="center"><img src="assets/logo-256.png" width="96" height="96" alt="Dynamic Island logo"></p>

# Dynamic Island for Windows

A macOS-style island for the top of your Windows 11 screen: notifications, calendar, pomodoro, reminders, notes, music and more, in a native Rust app (Win32 + Direct2D, no browser engine).

<p align="center">
  <img src="assets/screenshots/calendar.png" width="420" alt="Calendar page, month view">
  <img src="assets/screenshots/shortcuts.png" width="420" alt="Shortcuts page">
</p>

Dokumentasi bahasa Indonesia dan desain: lihat [`docs/`](docs/).

## Install

Windows 11, 64-bit. Open **PowerShell** (no admin needed) and run:

```powershell
irm https://raw.githubusercontent.com/hartono0297/dynamic-island/main/install.ps1 | iex
```

The script downloads the latest [release](https://github.com/hartono0297/dynamic-island/releases), checks its SHA-256, installs `dynamic-island.exe` (one file, about 1.2 MB, no runtime needed) to `%LOCALAPPDATA%\Programs\DynamicIsland`, adds a Start Menu shortcut and starts the island.

- **Update:** run the same line again. Your settings, notes and Google login are kept.
- **Without the script:** download `dynamic-island-<version>.zip` from Releases, extract it, right-click `install.ps1` → *Run with PowerShell*. The exe also runs from any folder.
- **Uninstall:**
  ```powershell
  irm https://raw.githubusercontent.com/hartono0297/dynamic-island/main/uninstall.ps1 | iex
  ```
  This removes the island's Claude Code hooks, autostart, shortcut and program folder. Your data stays; run `$env:DI_PURGE = '1'` first to delete it too.
- **SmartScreen:** the exe is not code-signed. If Windows warns about a downloaded exe, choose *More info → Run anyway*.

On first run, Windows asks for access to your notifications: allow it.

## Features

- **Always-visible notch** at the top centre, with a clock when idle. It hides only for fullscreen apps; clicks outside its shape pass through.
- **Hover to open the panel**, scroll the mouse wheel to change pages: **Calendar → Pomodoro → Reminders → Notes → Music → CLI → Shortcuts**.
- **Calendar:** Google Calendar with Day / Week / Month views; add and edit events. Recurring and multi-day events are read-only. Works offline from the last sync.
- **Pomodoro:** progress ring, start/pause/reset/skip, a small timer in the notch while it runs. Durations are set in Settings.
- **Reminders:** once, daily or weekly. Reminders that came due while the island was off fire at startup, marked "Missed".
- **Notes:** quick local notes, saved when the editor closes.
- **Music:** cover, title and controls for whatever is playing (Spotify, browser, ...), an animated equalizer in the notch, and optional synced lyrics in the notch from [lrclib.net](https://lrclib.net).
- **Notifications:** every Windows toast shows in the island; click one to open its app. Turn on Windows *Do Not Disturb* to avoid seeing them twice (Settings → General → *Turn off Windows pop-ups* shows how).
- **Claude Code:** "Claude finished" and "Claude needs you" notices through Claude Code hooks.
- **Shortcuts:** Screenshot, Clipboard, Emoji, Explorer, Calculator, Task Manager, Windows Settings, Lock screen.
- **CLI:** run one command at a time in PowerShell or cmd from a small terminal window under the panel, with history and `cd`.
- **Themes and languages:** Pill or Notch shape, six accent colours, Black or Graphite island, light/dark windows; Indonesian (default) or English.

## Settings

Open **Settings…** from the tray icon.

<p align="center"><img src="assets/screenshots/settings.png" width="560" alt="Settings window, General section"></p>

| Section | What's in it |
|---|---|
| Appearance | Island style (Pill / Notch), accent colour, island fill (Black / Graphite), window mode (Follow Windows / Dark / Light), clock when idle |
| General | Language (Indonesia / English), start with Windows, battery alert threshold, how to turn off Windows pop-ups, lyrics in notch, CLI shell (PowerShell / cmd) |
| Pomodoro | Focus, break and long break length; sessions before a long break |
| Integrations | Google Calendar client ID / secret, Save, Connect / Disconnect; install or remove Claude Code hooks |
| About | Version, source code, releases, data folder |

**Claude Code hooks:** Settings → Integrations → *Install hooks* adds `Stop`, `Notification`, `UserPromptSubmit`, `PostToolUse` and `SessionEnd` hooks to `%USERPROFILE%\.claude\settings.json`. Other hooks are left alone, and a backup is kept as `settings.json.bak-dynamic-island`. Also available as `dynamic-island.exe --install-hooks` / `--uninstall-hooks`.

## Setup Google Calendar

The Calendar page uses your own Google Cloud project (one-time setup):

1. In [Google Cloud Console](https://console.cloud.google.com/), create a new project.
2. **APIs & Services → Library** → **Google Calendar API** → **Enable**.
3. **Google Auth Platform** (OAuth consent screen) → **Get started**: any app name, your email, Audience **External**.
4. **Audience → Test users → Add users**: add the Gmail account whose calendar you want.
5. **Clients → Create client** → type **Desktop app**. Copy the **Client ID** and **Client secret** right away (the secret may be shown only once).
6. In the island: Settings → Integrations → paste both → **Save** → **Connect**.
7. In the browser, pick your account, continue past "Google hasn't verified this app", and **tick both permissions**.

While the project's publishing status is **Testing**, Google expires the login after 7 days and the island asks you to reconnect. To avoid that, choose **Audience → Publish app** (*In production*); no verification is needed for personal use, and the next login lasts.

## Data and privacy

Everything lives in `%APPDATA%\DynamicIsland\`:

| File | Contents |
|---|---|
| `config.json` | Settings, including the Google client ID / secret (plain text) |
| `notes.json`, `reminders.json` | Notes and reminders |
| `calendar-cache.json` | Last calendar data, for offline use |
| `google-token.bin` | Google login, encrypted with Windows DPAPI (only your Windows account can read it) |
| `cli-history.txt`, `cli-cwd.txt` | CLI history and folder |
| `lyrics-cache\` | Lyrics already fetched |

- Nothing is sent anywhere except Google Calendar (when connected) and lrclib.net (when lyrics are on: the artist, title and length of the playing track).
- Notes and reminders are local only; there is no sync and no telemetry.

## Build from source

1. [rustup](https://rustup.rs) with the default `x86_64-pc-windows-msvc` toolchain.
2. Visual Studio 2022 Build Tools with **Desktop development with C++** and a **Windows 11 SDK**.
3. From **PowerShell or cmd** (not Git Bash, whose `link.exe` shadows the MSVC linker):
   ```powershell
   cargo build --release
   .\target\release\dynamic-island.exe
   ```

The C runtime is linked statically (`.cargo/config.toml`), so the exe runs without the Visual C++ Redistributable. Quit the island first if it runs from `target\release`.

**Releasing:** push a `vX.Y.Z` tag. `.github/workflows/release.yml` builds the exe and publishes it with its checksum and a zip.

## Troubleshooting

- **No notifications in the island:** check Windows Settings → Privacy & security → Notifications, then run `& "$env:LOCALAPPDATA\Programs\DynamicIsland\dynamic-island.exe" --probe-notifications` to print what arrives for 60 seconds.
- **"Add a Google client ID first":** paste the client ID and secret in Settings → Integrations and press **Save**.
- **"Access blocked" / Error 403 from Google:** your account is not a test user (step 4), or the client is not a *Desktop app*.
- **"Google Calendar API isn't enabled":** enable it (step 2) in the same project as the client, wait a few minutes, connect again.
- **"Google permissions incomplete":** connect again and tick both permissions.
- **"config.json is invalid":** usually a missing or extra comma. The island moves the broken file to `config.json.bak` on its next save; copy your values back from there.
- **Island not visible:** it hides while an app is fullscreen; only one instance can run (check the tray).

## Limits

- Primary monitor only.
- No inline replies or action buttons on notifications; apps that don't use Windows toasts aren't seen.
- Notifications are polled every second (about 2–3% of one core) because the installed exe has no package identity. Builds from source can use `scripts/package.ps1` for event-driven mode.
- Calendar is Google only; events can't be deleted from the island, and recurring or multi-day events can't be created or edited.
- CLI commands get no input (stdin), so interactive programs won't work; one command runs at a time.
- The pomodoro timer is not kept when the island quits.

Design notes and plans: [`docs/superpowers/`](docs/superpowers/).
