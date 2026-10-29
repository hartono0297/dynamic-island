---
tags: [gotcha, logo, taskbar, installer, shell]
date: 2026-10-29
---

# The Windows 11 taskbar shows the exe's cached icon, not WM_SETICON

Symptom (v3.2.1): Settings' taskbar button still showed the old "eared tile"
although `assets/icon.ico` and the embedded exe icon were already the cat
head ([[logo]], [[studio-bata-logo-sources]]).

**Cause.** The taskbar button is not drawn from the window's icons. Windows 11
resolves the app's icon from the **exe path** (and the Start Menu shortcut
that targets it) through Explorer's icon cache. `vale.exe` has had the same
path since v3.1, so Explorer kept the v3.1 icon it cached for `vale.exe,0` and
`Vale.lnk`; the installer overwrote the exe in place and rewrote the shortcut
without telling the shell. `WM_SETICON` only reaches Alt+Tab.

**How it was proven.** A WinForms window in `powershell.exe` with
`Form.Icon` set to `assets/icon.ico`, then a capture of the taskbar: the
button showed the PowerShell exe icon, not the cat. A fresh `SHGetFileInfo` in
another process already returned the cat, so the disk cache was right and the
running Explorer's in-memory image list was stale.

**What Vale does about it.**
- `install.ps1` sets `IconLocation "$Exe,0"` on the shortcut and, before it
  starts Vale, calls `SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW|SHCNF_FLUSH)`
  for the exe, `Vale.lnk` and a pinned taskbar `Vale.lnk` (if any), then
  `SHCNE_ASSOCCHANGED`, then `ie4uinit.exe -show`. All best effort.
- The app does the same once per version at start
  (`ui::app_icon::refresh_shell_icon_once`): the stamp
  `<data dir>\shell-icon.txt` holds the last `CARGO_PKG_VERSION` that told the
  shell (`stamp_due`), which covers a manual copy over the old exe. No flush,
  so it never blocks the UI thread.
- The island's window class carries the shared class icons too
  (`app_icon::class_icons`), for Task Manager and other shell surfaces.

**What it cannot do.** Purge Explorer's in-memory cache without restarting
Explorer. The installer does not kill `explorer.exe` or delete
`iconcache_*.db` (too disruptive). A pinned taskbar icon may keep its own
cached copy until the user signs out and in; the installer says so only when a
pinned `Vale.lnk` exists. `SHCNE_ASSOCCHANGED` redraws every icon on the
desktop once (a short flicker), accepted once per install or version.

Rejected: an explicit AppUserModelID (it changes grouping and pins, and does
not bust the cache by itself).

Not verified live on a real taskbar: see the owner checks in
`docs/superpowers/plans/2026-10-29-v3.3.md`, Task 1.

Related: [[logo]], [[install-script-follows-latest-release]]
