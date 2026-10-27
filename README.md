<p align="center"><img src="assets/logo-256.png" width="96" height="96" alt="Dynamic Island logo: a dark pill with a cream dot on a terracotta tile"></p>

# Dynamic Island for Windows

A macOS-style island for the top of your Windows 11 screen: notifications, calendar, pomodoro, reminders, notes, music, a command line and more, in a native Rust app (Win32 + Direct2D, no browser engine). Version 3 brings the **Studio** look: warm terracotta and espresso colours, its own fonts and icons, and Bata, a small robot that lives in the notch.

<p align="center">
  <img src="assets/screenshots/calendar.png" width="420" alt="Calendar page in month view: October 2026 with today circled in terracotta, and today's events listed on the right">
  <img src="assets/screenshots/shortcuts.png" width="420" alt="Shortcuts page: eight tiles for Screenshot, Clipboard, Emoji, Explorer, Calculator, Task Manager, Windows Settings and Lock screen">
</p>

Design documents (in Indonesian) are in [`docs/design/`](docs/design/).

## Install

Windows 11, 64-bit. Open **PowerShell** (no admin needed) and run:

```powershell
irm https://raw.githubusercontent.com/hartono0297/dynamic-island/main/install.ps1 | iex
```

The script downloads the latest [release](https://github.com/hartono0297/dynamic-island/releases), checks its SHA-256, installs `dynamic-island.exe` (one file, about 1.7 MB, no runtime needed) to `%LOCALAPPDATA%\Programs\DynamicIsland`, adds a Start Menu shortcut and starts the island.

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
- **Calendar:** Google Calendar with Month, Week and Day views. The day view is an hour timeline with a line at the current time. Add and edit events in the island's own editor; repeating events, events over several days and events on calendars you can only view open read-only. Events can't be deleted from the island. Works offline from the last sync.
- **Pomodoro:** progress ring, start/pause/reset/skip, a small timer in the notch while it runs. Durations are set in Settings.
- **Reminders:** once, daily, or weekly on one or more days of the week, set in the island's own reminder editor with time shortcuts (*In 30 min*, *In 1 hour*, *Tonight*, *Tomorrow morning*) and a summary of when it rings next. Reminders that came due while the island was off fire at startup, marked "Missed".
- **Notes:** quick local notes, saved when the editor closes.
- **Music:** cover, title, a progress bar and controls for whatever is playing (Spotify, browser, ...), an animated equalizer in the notch, and optional synced lyrics in the notch from [lrclib.net](https://lrclib.net).
- **Notifications:** every Windows toast shows in the island; click one to open its app. Turn on Windows *Do Not Disturb* to avoid seeing them twice (Settings → General → *Turn off Windows pop-ups* shows how). A low-battery notice comes at the threshold set in Settings.
- **Claude Code:** "Claude finished" and "Claude needs you" notices through Claude Code hooks.
- **Shortcuts:** Screenshot, Clipboard, Emoji, Explorer, Calculator, Task Manager, Windows Settings, Lock screen.
- **CLI:** run one command at a time in PowerShell or cmd on the panel's CLI page. The island takes the keyboard only while you type: click the terminal to start (the panel stays open while you type). Esc hands the keyboard back to the app you were in; a click outside the island releases it to whatever you clicked. History suggestions (Tab, ↑), `cd`, `cls`, Stop / Ctrl+C and an enlarge button. A command keeps running while the panel is closed.
- **Bata, the assistant:** a small robot beside the clock and on the island's own notices (reminders, pomodoro, Claude Code, battery, greeting). It has faces for idle, happy, focus, listening, singing, alert, thinking, asleep, low and waving; it blinks, falls asleep after a while without activity and wakes when you point at the island. Other apps' notifications keep their own badge, and the open panel shows no robot.
- **Morning greeting:** once a day, before noon, Bata says good morning with the number of today's events and reminders, at startup, after a wake or when you unlock the PC.
- **Studio look:** the Bata palette (terracotta, espresso, cream) with six accent colours, Espresso or Graphite island fill, light/dark windows; DM Sans and DM Mono built into the exe, with titles and the clocks in DM Sans Semibold; the island's own vector icons. Pill or Notch shape. Indonesian (default) or English.

<p align="center">
  <img src="assets/screenshots/calendar-day.png" width="420" alt="Calendar day view: an hour timeline with two all-day events on top, a design review and a coordination meeting, and a terracotta line at the current time">
  <img src="assets/screenshots/notch.png" width="420" alt="Calendar week view in the Notch shape, whose top corners curve into the screen edge; each day is a column of event chips">
</p>

<p align="center">
  <img src="assets/screenshots/pomodoro.png" width="420" alt="Pomodoro page: a terracotta progress ring around 18:42 left in focus session 2 of 4, with reset, pause and skip buttons">
  <img src="assets/screenshots/reminders.png" width="420" alt="Reminders page grouped under Today, Tomorrow and a date, with a Missed badge on an overdue reminder and repeat icons on repeating ones">
</p>

<p align="center">
  <img src="assets/screenshots/notes.png" width="420" alt="Notes page: four note cards with a title, two lines of text and when each was last changed">
  <img src="assets/screenshots/music.png" width="420" alt="Music page: a sunset cover, the track title in italic serif, a progress bar at 1:42 of 3:58, playback controls and a Lyrics button">
</p>

<p align="center">
  <img src="assets/screenshots/cli.png" width="560" alt="CLI page inside the panel: git status, ls and git pull with their output, a done 2.1 s chip, the PowerShell shell, and the focused input with the hint that the panel stays open while you type">
</p>

<p align="center">
  <img src="assets/screenshots/editor.png" width="320" alt="Event editor in the dark window mode: title, date chip with Today, Tomorrow, Thursday and Next week shortcuts, start and end times with the duration, a notes field, Cancel and Save">
  <img src="assets/screenshots/reminder-editor.png" width="320" alt="Reminder editor: date and time chips, quick times In 30 min, In 1 hour, Tonight and Tomorrow morning, Once / Daily / Weekly with Monday, Wednesday and Friday picked, and the summary Every Mon, Wed, Fri at 09:00">
</p>

<p align="center">
  <img src="assets/screenshots/robot.png" width="460" alt="Bata's faces: idle, happy, focus, listening, singing, alert, thinking, asleep, low and waving, then a blink and the sizes from 48 to 20 px">
  <br>
  <img src="assets/screenshots/greeting.png" width="300" alt="Morning greeting notice: Bata waving beside Good morning, 3 events and 2 reminders today">
</p>

## Settings

Open **Settings…** from the tray icon.

<p align="center"><img src="assets/screenshots/settings.png" width="560" alt="Settings window, Appearance section scrolled down: accent colour, island fill, window mode, clock when idle, then the assistant card with Assistant in notch, Animation Full / Saver / Off, Sleep after 10 min and Morning greeting"></p>

| Section | What's in it |
|---|---|
| Appearance | A live preview of the island; island style (Pill / Notch), accent colour (Terracotta, Sage, Ochre, Clay, Olive, Plum), island fill (Espresso / Graphite), window mode (Follow Windows / Dark / Light), clock when idle. Then the assistant: assistant in notch (Bata on or off), animation (Full / Saver / Off), sleep after 1–60 minutes without activity, morning greeting |
| General | Language (Indonesia / English), start with Windows, battery alert threshold, how to turn off Windows pop-ups, lyrics in notch, CLI shell (PowerShell / cmd) |
| Pomodoro | Focus, break and long break length; sessions before a long break (locked while a pomodoro runs) |
| Integrations | Google Calendar: status, client ID / secret, Save, Connect / Disconnect, setup guide. Claude Code: install or remove hooks |
| About | Version, source code, releases, data folder |

**Animation:** *Full* blinks, lifts Bata when you point at it and bounces on alerts. *Saver* drops the blinking and motion but keeps the faces; Full also behaves like Saver while Windows Energy Saver is on. *Off* runs no robot timers at all: the faces still follow what happens, but Bata does not fall asleep. With the assistant off, the island looks and works as without Bata.

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
   cargo build --release --locked
   .\target\release\dynamic-island.exe
   ```

The C runtime is linked statically (`.cargo/config.toml`), so the exe runs without the Visual C++ Redistributable. Quit the island first if it runs from `target\release`. Tests: `cargo test --locked`.

**Screenshots:** the images above come from the debug build's demo modes (`--demo`, `--demo-robot`, `--demo-cli`, `--demo-editor`, `--settings`). With no island running, `cargo build` and then `.\scripts\capture-screenshots.ps1` regenerates them in English.

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
- Calendar is Google only; events can't be deleted from the island, and repeating events, events over several days and events on read-only calendars can't be edited. New events are single-day.
- CLI commands get no input (stdin), so interactive programs won't work; one command runs at a time.
- The pomodoro timer is not kept when the island quits.

## Fonts / credits

The exe embeds Latin subsets of two typefaces, both under the SIL Open Font License 1.1:

- **DM Sans** by Colophon Foundry for Google Fonts, © 2014 The DM Sans Project Authors (text and numbers)
- **DM Mono** by Colophon Foundry for Google Fonts, © 2020 The DM Mono Project Authors (the CLI)

Licence texts and copyright lines are in [`assets/fonts/OFL.txt`](assets/fonts/OFL.txt). [`scripts/make-fonts.ps1`](scripts/make-fonts.ps1) rebuilds the files from pinned upstream sources.

Design notes and plans: [`docs/superpowers/`](docs/superpowers/).
