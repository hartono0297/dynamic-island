---
tags: [decision, shelf, ole, drag-drop, v3.2]
date: 2026-10-28
---

# File shelf (v3.2 Run 11)

`Page::Shelf`, right after Clipboard (`Page::ALL` is eleven pages; a saved
`panel_pages` without "Shelf" gets it back after Clipboard like any new page).

- **List** (`src/shelf.rs`, pure): paths only, never copies, in **memory
  only** (a restart empties it: decided, a persisted shelf would need stale
  handling across restarts and a privacy story). Max 12, deduped by path
  ignoring letter case and a trailing slash, extras ignored. `cards(probe)`
  gives name (`short_name`, cut to 14 chars keeping a short extension), icon
  kind (by extension, folder by probe) and `stale` (path gone), with the file
  system injected (`probe`) so tests never touch real files.
- **Drop** (`src/shelf_ole.rs`): `RegisterDragDrop` on the island window
  after `OleInitialize` (the thread was already `CoInitializeEx` STA from
  `app::run`; OLE just adds drag and drop on top). `DropTarget`
  (`#[implement(IDropTarget)]`, needs the `windows-core` crate as a direct
  dependency and the `Win32_System_SystemServices` feature) accepts only
  `CF_HDROP`. Effect answered: link if offered, else copy, **never move**
  (`choose_effect`, tested). It posts `Event::ShelfDrag(true/false)` and
  `Event::ShelfDrop(paths)` on the app channel + `WM_APP_EVENT`; nothing
  re-enters `with_app` from inside OLE.
- **Reducer**: `ShelfDrag(true)` opens the panel on the shelf and holds it
  (`shelf_held`, like `clip_held`: hover takes over, it ends with
  `ShelfDrag(false)`, a page change or a collapse); not in fullscreen. A
  drop makes `App` add the paths and hold 2.5 s (`TIMER_SHELF_HOLD` 27) so
  the cards are seen arriving. Clicks (`Hit::ShelfCard/ShelfRemove/
  ShelfClear`) only count while the page is open; a card collapses the panel
  then opens its file (`Action::Shell`), a missing file is only marked.
- **Drag out**: a left press on a card whose file is there, then a move of
  more than 6 dip with the button down, calls `shelf_ole::drag_out` from
  `wnd_proc` outside the app borrow (`DoDragDrop` blocks and pumps messages).
  The data object is the shell's own for that item
  (`SHCreateItemFromParsingName` -> `BindToHandler(BHID_DataObject)`), offered
  for copy or link only; the card stays after the drop. The click that would
  have opened the card is dropped (`click.clear()`).
- Stale detection runs when the page opens (`sync_shelf`), after a drop or
  a change, and before a card is opened or dragged: no timer, no IO while
  the page is closed.
- Layout: four cards a row, up to three rows, an X on each card's corner
  (first in `regions`), "Kosongkan" beside the heading, an empty-shelf hint.
  Demo scene `shelf` (made-up names, one stale) and a `shelf` shot.

**Not verified live:** that OLE accepts drops on this layered, no-activate
window (and where: layered windows ignore fully transparent pixels, so only
the drawn island takes a drop), that the idle island is big enough to aim at,
that a drag hovering opens the panel in time, `DoDragDrop` out to Explorer and
other apps (and the effect each app picks), a mouse that stays still after a
drop (the 2.5 s hold is the fallback), and stale detection for network or
removable paths.

Related: [[mixer]], [[open-mode]].
