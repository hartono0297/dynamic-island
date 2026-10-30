---
tags: [decision, search, hotkey, v3.2]
date: 2026-10-28
---

# Quick search (v3.2 Run 10, "Pencarian-lite")

A Spotlight-like search that opens on a hotkey. **Settings → Umum →
Pencarian** (`config.search_hotkey`, `SearchHotkey`: `CtrlAltSpace` is the
default since 3.2.1 (it was `Off` in 3.2.0, so nobody found it), `WinAltS`,
`Off`; a file that never names it gets the default, "Off" stays off, a typo
reads as `Off`, Alt+Space is never offered). The key is saved as
`quick_search_hotkey`: 3.2.0 wrote `search_hotkey: "Off"` into every config, so
that old key is ignored when it says "Off" (never chosen) and honoured for
any other value; the Settings control is island-owned (`merge_owned`), so a
choice, Off included, sticks across saves (3.2.1 review fix). Registered as `RegisterHotKey` id 2 (the Clipboard's is 1) in
`app_search.rs`; a failed registration shows "Sudah dipakai" in Settings.

- **Where it lives:** `Page::Search` is a page of the panel but not one of
  `Page::ALL`: no wheel, no dots, no slide, never saved, and
  `follow_page` / `wheel` / `leave_hidden_page` stay out of it
  (`PanelPages::position` would panic for it). The reducer keeps
  `search_open` + `search_prev`; the page it came over returns when it
  closes (post-process in `apply`, which also emits `ReleaseSearchFocus`).
- **Reducer events:** `SearchHotkey` (open, asks `FocusSearch`; again =
  close), `SearchClose`, `Click(Hit::SearchResult(i))` (closes and emits
  `SearchRun(i)`; Enter is that same click, so key and mouse share a path).
  Results themselves live in `App` (`SearchState`), the reducer stays pure.
- **Field:** a third hidden EDIT (`cli_input::ID_SEARCH`, like the CLI and
  Clipboard ones: IME, paste, undo) drawn by `paint_text_field`, which the
  Clipboard search now shares. Blur (`WM_ACTIVATE` inactive or the EDIT losing
  focus) closes the search; if the island cannot get the keyboard, it closes
  at once.
- **What it finds (`src/search.rs`, pure):**
  - apps: `.lnk` files under the all-users and user Start Menu `Programs`
    folders, 4 levels deep, one per name, uninstallers/help skipped; indexed
    once on a thread (`Event::SearchIndex`), only after the first open.
  - settings pages: a fixed table of 16 `ms-settings:` pages with ID/EN
    names.
  - calculator: a recursive-descent parser (`+ - * / % ^`, `× ÷`, brackets,
    `.` or `,` decimals), no variables or functions. The language's
    thousands mark (`.` in Id, `,` in En) counts as one when exactly three
    digits follow it, before any decimal mark ("1.000 + 500" = 1500 in Id);
    otherwise either mark is a decimal one. Division by zero, a
    non-finite result, > 120 chars or > 48 nested levels → no result. A bare
    number is not a calculation. Enter copies the result (own clipboard
    write, so the history does not take it twice) and shows "Disalin".
  - web: `? text` (alone) or the last row of any query: an
    `https://www.google.com/search?q=` address with the text percent-encoded;
    `is_web_url` checks it again before it is run. No other scheme is ever
    opened from a typed line.
  - ranking: prefix (3) > word start (2) > substring (1), stable within a
    score, an app wins a tie with a settings page; at most 6 rows, the
    calculation first and the web row last.
- **Running:** `shortcuts::Action::Shell(String)` (`ShellExecute open`) after
  the panel collapses, like a tile. For an app, a settings page or the web
  the keyboard is *not* handed back to the window from before (`cli_prev`
  cleared, so the release only drops the field's focus) and
  `AllowSetForegroundWindow(ASFW_ANY)` is called: the shell runs while the
  island still has the foreground, so the launched window comes to the
  front instead of opening behind. A copied calculation hands it back.
- Demo scenes `search` (a made-up list of apps, never the real Start Menu) and
  `search-files` (made-up files, folders and drawn icons).

**Not done / later:** recent/frequent ranking, a "Run command" row
(deliberately left out: only apps, settings pages, files and folders, a
calculation and the web are ever run).

**Not verified live:** the hotkeys' registration conflicts on other PCs,
typing with an IME in the field, foreground/focus handling (Win+Alt+S while
another app is full screen is ignored by design), launching real Start Menu
apps, a very large Start Menu.

Related: [[shortcut-tiles]], [[gestures]].

## More ways in (v3.2.1)
- Tray menu: **Cari…** / **Search…** (`MenuCmd::Search`, second item), shown
  whatever the hotkey says (off or taken); it runs `App::on_search_hotkey`.
- Pintasan tile **Cari** (`Shortcut::Search`, after Clipboard in
  `Shortcut::ALL`; old tile lists get it back after Clipboard): the reducer
  opens the search like the hotkey (`search_hotkey()`), only from an open
  panel.
- `focus_search` starts the Start Menu index for every way in.
- Nine built-in tiles now: the Pintasan page has three rows (4 + 4 + 1).

## 3.2.1 review fixes
- **Another page beneath it closes it:** the Clipboard hotkey, files dragged
  over the island, a Mole strip click change the page under an open search;
  the post-process in `apply` (now run first) closes it when
  `search_open && (page != Search || !panel_open())`, emits
  `ReleaseSearchFocus` and keeps the new page.
- **From the CLI input or the Clipboard field:** `search_hotkey` clears
  `cli_focused` / `clip_focused` without their release effects (those ran
  after `FocusSearch` and handed the keyboard to the old window, which closed
  the search at once); `cli_prev` stays.
- **Tray "Cari…"** sends `Event::SearchOpen`: opens, or re-focuses an open
  search (never closes it). The window it returns to is the one in front
  when the menu opened, unless that is the taskbar / tray overflow (by
  class) or the island; then the topmost ordinary window in the z-order.
- **Hotkey retry:** a failed `RegisterHotKey` (search and Clipboard) is
  tried again on every sync (config read) and when the tray menu or
  Settings opens (`retry_hotkeys`).
- No search EDIT (creation failed at start): `FocusSearch` closes it again.


## Round 2 review (2026-10-29)

Calculator: a thousands mark only follows one to three digits that do not
start with 0, then groups of exactly three ("0.001" is a decimal, "1234.567"
is not grouped, "1.000.000" is). Custom tile paths also accept `C:/Tools/a.exe`
(a drive letter then `\` or `/`). The OAuth loopback ignores a code or error
whose state is not this login's (404, keeps waiting). Hapus no longer leaves
the screenshot's own file in `taken` while it waits for it. The System CPU
figure eases with its own 0.5-point step (it is in percent).

## Files and folders (v3.3)

File search: see [[search-files]]. What changed here: two more kinds
(`Kind::File`, `Kind::Folder`, `Action::OpenPath`), `search(line, apps,
files, lang, labels)` with files matched from two typed characters, the row
order calculation, apps/settings, files/folders, web (at most 6 rows, 2 apps
kept when apps match), a `path` on each result for the details worker (icon,
folder count, app install size, filled in after the rows show) and the
highlight staying on the same result when an index arrives (`keep_selection`).
The hint now says "Cari aplikasi, berkas, hitung…".

## Back button (2026-10-30)

A back arrow (`Icon::Back`, `Hit::SearchBack`, `SearchLayout::back`, 32 px
wide, the field's height) sits left of the field. Its click runs the same
reducer path as Esc (`close_search`, used by `Event::SearchClose` too): the
page returns and `ReleaseSearchFocus` hands the keyboard back. The island
window answers `WM_MOUSEACTIVATE` with `MA_NOACTIVATE`, so the click does not
blur the EDIT before the reducer closes it. Label key `SearchBack` (Kembali /
Back); the island has no tooltip mechanism.
