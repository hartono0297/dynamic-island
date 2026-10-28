---
tags: [decision, cleaner, system, licence]
date: 2026-10-28
---

# The cleaner ("Bersihkan", mode "Pilih sendiri")

Run 5 of the v3.1 plan (keputusan-v3.1 decision 4b, option D;
`docs/design/deepclean-opsi-d.html`). Engine in `src/clean.rs`, the panel in
`panel_layout::cleaner_layout` and `render_panel::paint_cleaner`.

- **Seven categories** (`CleanCat`, row order): temp & logs (user folders
  only, files > 7 days), browser caches (Chrome, Edge, Firefox, Brave,
  Opera), app caches (VS Code, Teams, Discord, Spotify, Zoom), developer
  caches (npm, pip, NuGet, Go, Cargo), Autodesk & Adobe, old installers
  (`.exe`/`.msi` at the top of Downloads, > 30 days), Windows Update
  leftovers. The top three are ticked when it opens.
- **Locations are a data table** (`LOCATIONS`), injectable roots (`Roots`),
  so `select(roots, cats, now, running)` is testable on a temp tree under
  `target/clean-tests`. Tests never touch the real Temp, Downloads, AppData
  or browser folders and never call `Roots::known()`.
- **Never follows links**: any reparse point (symlink, junction) is skipped,
  in the path to a location and inside it.
- **Open browsers are skipped** by exe name, without case, checked at the
  scan and again right before deleting; they are named in the row and the
  notice.
- **How files go**: caches permanently, file by file; a locked or denied
  file is counted once, never retried. Installers go to the Recycle Bin via
  `IFileOperation` (`FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE`, plus
  `FOF_WANTNUKEWARNING` so a file too big for the bin asks instead of being
  destroyed). COM is initialised (STA) on the cleaner thread.
- **Admin without elevating the island**: Windows Update runs this exe again
  with `--clean-admin windows-update` through `ShellExecuteExW` verb
  `runas`. The helper (in `main`, not in `opens_windows`, no window) deletes
  only `%WINDIR%\SoftwareDistribution\Download` and exits with the number of
  skipped files (0..=250) or 255. A refused UAC prompt is "skipped".
  `app.manifest` stays asInvoker.
- **Confirm in the button**: the first click arms it ("Yakin? Hapus X") for
  `CLEAN_CONFIRM` (4 s, the reducer's `next_deadline`, so the existing
  one-shot `TIMER_DEADLINE`); the second click inside that sends
  `Effect::CleanDelete`. Any other click, a tick, closing the panel or the
  timeout disarms it. Reducer tests prove a single click never deletes.
- **Result = measured free space**: `GetDiskFreeSpaceExW` on the affected
  drives before and after, on the worker. The notice (`NoticeKind::CleanDone`,
  Bata Happy, a sage sparkle badge when the robot is off) shows the gain,
  the drive's free space now and what was skipped. The list scans again
  if the cleaner is still open.
- **Sub-view, not a page**: the reducer keeps `cleaner: Option<CleanerUi>`
  while the page is Sistem and the panel open; a page change, the panel
  closing or the back button ends it. The panel grows to
  `ShapeKind::PanelClean` (560 × 352, the tall canvas). No page dots. The
  System page stops sampling while the cleaner covers it.
- **Power**: no timers or redraws while it is closed; scanning shows a
  static "memindai…", no spinner. A clean keeps running when the panel
  closes; its notice still comes.
- **Not here (by decision)**: no Recycle Bin category, no automatic or
  scheduled cleaning.
- **Two modes** (Run 6): the header segment switches to "Otomatis · Mole"
  (not while either mode cleans); the mode last used is remembered
  (`clean_mode`). See [[mole-mode]].
- **Done-notice body** is fitted to the 360 px notice by measure
  (`fits`), shorter wordings before dropping parts; see [[mole-mode]].

## Mole: data only, and which licence

The location list follows Mole for Windows by tw93 (`lib/clean/*.ps1` on
branch `windows`). Gotcha: that branch was **relicensed to GPL-3.0 on
2026-10-11** (commit 5b7695bb). The data is taken from the last MIT commit,
500dfc4c (Mole 1.30.0), and only the data (folders, age rules); the Rust is
the island's own. `NOTICE` carries the MIT text with its copyright line
("Copyright (c) 2025 tw93"); README credits it. Never read or copy Mole's
macOS code. Where the island is narrower than Mole (Teams databases, Zoom
settings, NuGet packages, no system temp) the table says so.

## Verify pass (2026-10-28)

- **Prune only what the clean emptied** (`prune_emptied`): walk up from each
  deleted file to its location folder. Pruning every empty folder under a
  location deleted the user's own empty folders across the whole profile and
  %APPDATA% (the profile-root and Roaming-root `*.log` locations), even an
  empty git `refs`.
- **Temp must be a real temp folder** (`safe_temp`): without TMP/TEMP,
  GetTempPath returns the profile or Windows, and "older than 7 days" there
  would take documents. A Temp root that is not named Temp/Tmp, holds the
  profile, AppData, Downloads or Windows, or is Windows' own, is left out.

Related: [[mole-mode]], [[system-page]], [[bata-robot-expressions]],
[[bata-idle-power-budget]], [[studio-icons]], [[screenshot-capture-script]],
[[studio-tabular-figures-word-gaps]]

Listed in [[index]].
