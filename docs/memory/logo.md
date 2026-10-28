---
tags: [decision, logo, assets, v3.2.1]
date: 2026-10-29
---

# The cat head is the logo everywhere (v3.2.1)

The old "eared tile" (tile, ears, pill, dot) is gone. The mark is now the
notch cat's head: a terracotta flat-based dome, two ears, two plain espresso
pill eyes; geometry and the three files to keep in step are in
[[studio-bata-logo-sources]].

Where it shows, and what loads it:
- **exe, Explorer, taskbar, Alt+Tab, tray:** the embedded `1 ICON
  "assets/icon.ico"` (`app.rc`). The tray asks `ui::app_icon` for the small
  size of the system DPI (`LoadIconWithScaleDown`), so the 16-24 px frames
  matter; the Settings window icons use the same resource.
- **Settings sidebar and About:** drawn in code (`fn logo`).
- **README header, site, Store package:** `assets/logo-256.png`,
  `site/img/logo.svg`, and `scripts/package.ps1` scales the PNG.

`scripts/make-icon.ps1` rebuilds the .ico and PNG (9 sizes, 8x supersampled).

**Windows caches icons.** After installing, the pinned shortcut, the exe in
Explorer and the tray may still show the old mark: a running Vale keeps its
old tray icon until it restarts (the installer starts a fresh one), and
Explorer's icon cache can need `ie4uinit.exe -show` or restarting Explorer.
Not verified live on a real tray.
