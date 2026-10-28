---
tags: [decision, search, hotkey, v3.2]
date: 2026-10-28
---

# Quick search (v3.2 Run 10, "Pencarian-lite")

A Spotlight-like search that opens on a hotkey. **Settings → Umum →
Pencarian** (`config.search_hotkey`, `SearchHotkey`: `CtrlAltSpace` is the
default since 3.2.1 (it was `Off` in 3.2.0, so nobody found it), `WinAltS`,
`Off`; a file that never names it gets the default, "Off" stays off, a typo
reads as `Off`, Alt+Space is never offered; a config the island saved under
3.2.0 holds an explicit "Off" and keeps it). Registered as `RegisterHotKey` id 2 (the Clipboard's is 1) in
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
    `.` or `,` decimals), no variables or functions; division by zero, a
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
  the panel collapses, like a tile.
- Demo scene `search` (a made-up list of apps, never the real Start Menu).

**Not done / later:** file search (Windows Search on a thread with a time
limit), recent/frequent ranking, a "Run command" row (deliberately left out:
only apps, settings pages, a calculation and the web are ever run).

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

