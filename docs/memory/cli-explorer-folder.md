---
tags: [decision, cli, explorer, folders]
date: 2026-10-27
---

# CLI follows File Explorer, and its folder menu

Run 3 of the v3.1 plan (keputusan-v3.1.html p2). Pure logic in
`src/cli_folders.rs`; the COM reader in `src/sources/explorer.rs`, see
[[cli-explorer-com]].

- **When it reads.** Only on the transition into `View::Panel(Page::Cli)`
  (`App::follow_cli_page`, around each reducer step) and when the folder
  menu opens. Never per redraw. One read at a time; a request made while
  one runs is queued (and remembers if it was for the page opening). The
  page-open read is skipped in "Folder terakhir" mode or once pinned.
- **When it switches.** Only the page-open read may move the CLI, and only
  if `choose_on_open(config.cli_start, running, pinned, …)` says so,
  checked when the result arrives (a command may have started meanwhile)
  and only while the CLI page still shows. A switch sets `cli_cwd`, shows
  the "dari Explorer" chip and adds the quiet `↳ pindah ke …` line.
- **Pinned until the panel closes.** A manual `cd` or a menu pick sets
  `cli_pinned` and clears the Explorer chip; it resets when the panel
  closes (not on a page change).
- **cli-cwd.txt stays the user's own.** An automatic switch does not write
  it. `cd`, a menu pick and starting a command do, and push the folder to
  recent. So "Folder terakhir" mode is always a folder the user chose or
  ran something in.
- **Never mid-command.** Auto-switch and menu picks check
  `cli.running()`; the folder chip is not clickable while a command runs
  (`CliHead::folder_menu`), and a command starting closes an open menu.
- **Files** (next to `cli-cwd.txt`, written with `cli::write_atomic`):
  `cli-favorites.txt` one folder per line, in starred order, at most 20;
  `cli-recent.txt` `<unix secs>\t<folder>` per line, newest first, at most 5
  (a line without a time still loads). Folders are de-duplicated without
  case and lose a trailing `\` (not on `C:\`). Gone folders are hidden in
  the menu, never deleted from the files.
- **Menu.** Sections: Explorer windows (z-order, first "aktif terakhir"),
  Favorit (hint "klik kanan = lepas" on the first), Terakhir dipakai (not
  repeating favourites). Other Explorer rows have no note: there is no
  honest "last active" time for them (the design's "2 mnt lalu" there is
  left out). Explorer and recent rows have a star button (outline; filled
  for a favourite). Layout trims to the terminal's bottom, at most 3
  Explorer rows in the normal panel (6 enlarged), never a header without
  its first row. Right-click on a favourite row unstars it (app.rs
  `WM_RBUTTONUP` hit-tests first; elsewhere the tray menu as before).
- **Chips.** "dari Explorer" is chip 0 when shown; the folder chip after it
  is the one that gives way, and "dari Explorer" is dropped (width 0) when
  the folder would be cut below 120 px (the Ended state with a result).

Keyboard focus is untouched by the menu: [[cli-panel-keyboard-focus]].
Settings → Umum gained "Folder awal CLI" (7 rows end at y 432). Since
[[system-page]] added a warnings card under them, General scrolls too, like
[[settings-appearance-scroll]].
Demo scenes `cli-from-explorer` and `cli-folder-menu` use a fixed "now"
([[demo-data-time-of-day]]); shots in [[screenshot-capture-script]].

Listed in [[index]].
