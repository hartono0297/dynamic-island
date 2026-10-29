<p align="center"><img src="assets/logo-256.png" width="88" height="88" alt="Vale logo: a terracotta cat head with two ears and two dark pill eyes"></p>

<h1 align="center">Vale</h1>

<p align="center">A notch for the top of your Windows 11 screen.<br>Music, calendar, notes, clipboard, volume and more, one hover away.</p>

<p align="center">
  <a href="https://github.com/hartono0297/dynamic-island/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/hartono0297/dynamic-island?style=flat-square&color=c8644a"></a>
  <a href="https://github.com/hartono0297/dynamic-island/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github/stars/hartono0297/dynamic-island?style=flat-square&color=c8644a"></a>
  <a href="LICENSE"><img alt="MIT license" src="https://img.shields.io/github/license/hartono0297/dynamic-island?style=flat-square&color=c8644a"></a>
  <img alt="Windows 11" src="https://img.shields.io/badge/Windows-11-c8644a?style=flat-square">
  <img alt="Rust, about 2 MB, no runtime" src="https://img.shields.io/badge/Rust-2%20MB-c8644a?style=flat-square">
</p>

<p align="center"><img src="assets/screenshots/tour.png" width="560" alt="Animated tour of Vale's pages: Calendar, Pomodoro, Reminders, Notes, Music with lyrics, Mixer, Clipboard, File shelf, System, CLI, Shortcuts and Search"></p>

## Install

Open PowerShell (no admin needed) and run:

```powershell
irm https://raw.githubusercontent.com/hartono0297/dynamic-island/main/install.ps1 | iex
```

It installs the latest [release](https://github.com/hartono0297/dynamic-island/releases/latest) (one file, about 2 MB, no runtime) and starts it. Run the same line again to update. Remove it with [`uninstall.ps1`](uninstall.ps1).

## What it does

Hover the notch to open it, scroll to change pages.

| | |
|---|---|
| **Music** | Cover, controls, progress and synced lyrics in the notch. Scroll over the notch for volume, swipe or middle-click to skip. |
| **Mixer** | Master and per-app volume sliders. |
| **Clipboard** | Searchable history of text, links, colours, files and images, with pins. |
| **Screenshot** | A preview right after you capture, with Copy, Save or Delete. |
| **File shelf** | Drop files on the notch, drag them out later. |
| **Search** | `Ctrl+Alt+Space`: apps, calculator, Windows settings, web. |
| **Calendar** | Google Calendar with day, week and month views. |
| **Reminders, Notes, Pomodoro** | Quick ones, in the panel. |
| **Notifications** | Windows toasts and [Claude Code](https://claude.com/claude-code) prompts in the notch. |
| **System** | CPU, memory, disks, top apps and a safe cleaner. |
| **CLI** | A small terminal that follows your Explorer folder. |
| **Shortcuts** | Your own tiles for apps, folders and links. |

<p align="center"><img src="assets/screenshots/notices.png" width="460" alt="Animated notices in the notch: a greeting, a copied item, a screenshot preview, a disk warning and a long lyric line"></p>

Everything is configurable in **Settings** (tray icon): open mode (hover or click), page order, accent colours, shortcuts, hotkeys, language (Indonesian and English). It can also stay behind your windows (experimental).

## More

- **[Guide](docs/guide.md)**: every feature and setting, Google Calendar setup, data and privacy, troubleshooting.
- **Private by design**: notes, reminders and history stay on your PC; nothing is sent except Google Calendar and, with lyrics on, the track name to [lrclib.net](https://lrclib.net). There is no telemetry.
- **Build**: Rust stable and the MSVC toolchain, then `cargo build --release --locked`. Details in the [guide](docs/guide.md#build-from-source).

## License

[MIT](LICENSE) © 2026 hartono0297. The typefaces and the cleaner's location list keep their own licences; see [NOTICE](NOTICE) and the [guide](docs/guide.md#fonts-and-credits).
