---
tags: [decision, search, files, v3.3]
date: 2026-10-29
---

# Search: files and folders (v3.3)

The quick search ([[search]]) also finds file and folder **names** under the
user's Desktop, Documents, Downloads, Pictures, Videos and Music, with icons,
sizes, folder counts and app install sizes filled in from a background worker.

## Where the names come from

- **Our own walker** (`src/search_files.rs`), not Windows Search: indexing may
  be off or leave folders out, OLE DB from Rust is heavy, and a walker is
  testable on temp trees (the tests walk `target/search-files-tests`, never a
  real folder; `default_roots` is the only part that asks Windows and no test
  calls it). Names only: contents, other drives and other folders are not
  searched, on purpose.
- Roots come from `SHGetKnownFolderPath` (`FOLDERID_Desktop`, `Documents`,
  `Downloads`, `Pictures`, `Videos`, `Music`), so a redirected or OneDrive
  folder is whatever Windows says. `dedupe_roots` drops a root inside another
  (Pictures inside a redirected Documents) or a repeat; the outer one walks it
  once, and the walk never enters a folder that is a root.
- **Breadth first per root**, so shallow (likely) items are kept before a cap
  bites. Folders list in name order, so a cap cuts the same entries each time.
- **Limits** (constants, easy to tune): `FILE_DEPTH` 5 (the root and four
  folders below it are read: `Documents\A\B\C\D\file` is found, `...\E`'s
  contents are not; deeper is mostly code trees and archives),
  `FILE_ROOT_MAX` 10 000 entries per root (a huge Documents cannot starve
  Desktop or Downloads), `FILE_TOTAL_MAX` 30 000 overall (about 6 MB of paths
  and a walk of a few hundred ms on an SSD).
- **Skipped:** hidden or system entries, names starting with `.`,
  `node_modules`, `__pycache__`, `$RECYCLE.BIN`, `.lnk` files (apps come from
  the Start Menu).
- **Cloud and links:** a link (a name-surrogate reparse point: symlink or
  junction, `FileType::is_symlink` on Windows) is skipped, file or folder, and
  never followed. Other reparse points are **not** links: OneDrive marks its
  folders with a cloud reparse tag, so "skip every reparse folder" (the first
  cut) dropped every subfolder of a OneDrive Desktop/Documents (fixed in the
  v3.3 review, `visit`, test `cloud_folders_are_walked_but_links_are_not`). A
  folder with `FILE_ATTRIBUTE_RECALL_ON_OPEN` (listing it would make the
  provider download the listing) is listed but not entered.
  A file that is a cloud placeholder is just a name in the listing. Files are
  never opened, so nothing is hydrated; the icon is `SIIGBF_ICONONLY`, never a
  content thumbnail.
- **When:** every way in (hotkey, tray, tile: `focus_search`) starts a
  short-lived thread `vale-search-files` (background thread mode) if no walk
  is running and the last one started more than `FILE_FRESH` (30 s) ago; the
  old list answers meanwhile. The result comes back as `Event::SearchFiles`.
  **In memory only**, never written to disk, nothing runs while the search is
  closed ([[bata-idle-power-budget]]).

## Matching and rows (`search.rs`, pure)

- Apps and settings match from the first character; files and folders from
  `FILE_MIN_CHARS` = 2 (trimmed, counted in characters). A `?` line is still
  the web only.
- Files rank by `match_score` on the lower-cased name (the index keeps
  `lower`, so a keystroke lowers none of 30 000 names), then **folders before
  files**, then newer `modified`, then the shorter name.
- Still six rows at most (`MAX_RESULTS`, the layout's limit). Order:
  calculation, apps/settings, files/folders, web. `split_room`: files take at
  most `FILE_ROWS_MAX` = 3, never so many that fewer than `APP_ROWS_MIN` = 2
  app rows are left when apps match; apps take the rest; room left over goes
  to the files that did not show yet. So files only: up to 5 rows; apps only:
  up to 5; both: 2 apps and 3 files. Grouped, not interleaved, so the keyboard
  path to apps stays predictable.
- `Action::OpenPath(PathBuf)` runs through `shortcuts::Action::OpenPath` (as a
  shelf card does), after `shelf::probe` says it is still there (it never
  probes a network path on the UI thread), with `AllowSetForegroundWindow`
  like an app. Running a file from Downloads runs it, as a double-click would.
- Subtitle: file `Unduhan › Proyek · 2,4 MB`, folder `Dokumen › Proyek`; a
  deeper folder shows only the nearest one (`Dokumen › … › Sub`) because the
  subtitle gets 40 % of the row. `search_file_size`: 1 KB at the least, KB
  below 1 MB, one decimal MB below 10 MB, then `format_size`.
- `keep_selection(old, at, new)`: when the app index, the file index or a
  re-search arrives the highlight stays on the same result (compared by its
  action), else row 0. Typing a new query still goes to row 0.

## Details in the background (`src/search_details.rs`)

- One parked worker thread `vale-search-details`, started the first time the
  search opens, COM single-threaded apartment, blocks on its channel (free
  while idle). A request carries a `generation`; the worker jumps to the
  newest request, also between items, and `Details::accept` drops an answer
  from an older generation. Answers are `Event::SearchDetail` + `WM_APP_EVENT`.
- What it reads: the **icon** (`IShellItemImageFactory::GetImage` 32 px,
  icon only, `GetDIBits`, premultiplied BGRA; a bitmap with no alpha anywhere
  is made opaque), a folder's **item count** (direct children that are not
  hidden or system, capped at 999: "999+"; not for a recall-on-open folder),
  an app's **install size** (the `.lnk` target by `IShellLinkW` without
  resolving it, matched with an Uninstall entry of HKLM 64-bit, HKLM 32-bit
  or HKCU whose `InstallLocation` holds the target or whose `DisplayIcon` is
  the target; `EstimatedSize` in KB; shown only when found; a location that
  is a drive root or Program Files itself never matches). The app icon is the
  target's, not the shortcut's (that would carry the arrow).
- `App` keeps `Details` (a map by path, at most 256, emptied when it grows
  past that, and the shown rows are asked for again) and `wanted_details`
  asks only for shown rows that have none. An answer redraws the search; it
  never re-ranks. Rows show at once with a glyph (`Icon::Grid` app,
  `Icon::Folder`, the shelf's icon kind for a file) and the icon replaces it.
  The renderer keeps one D2D bitmap per path (`search_icons`, cleared with
  the target, rows no longer shown are dropped each frame).
- Tried on the dev machine once (throw-away, not a test): 205 Start Menu apps
  gave 205 icons, 189 resolvable targets and 36 install sizes in about 2 s
  off the UI thread; most Windows' own apps have no Uninstall entry, so no
  size is normal.

## Not verified live

A real OneDrive Documents with online-only files (nothing should download),
a very large Documents, a shell extension that hangs `GetImage` (the worker
would stall, the UI keeps glyphs, and it recovers when the call returns),
the keyboard path with many rows, and light/dark looks of the shell icons.
The "Membaca berkas…" hint only shows when there are no rows at all, which
needs a line the web row cannot take (a control character): the web row is
always there otherwise, and the layout has no spare row.

Related: [[search]], [[shelf]] (icon kinds, `OpenPath`), [[cleaner]] (temp-tree
tests, reparse rules), [[screenshot-capture-script]], [[clipboard-privacy]].
