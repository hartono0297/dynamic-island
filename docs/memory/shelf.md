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

## v3.2.1: opening on a drag, and a tidier page (2026-10-29)

**Checked with a real OLE drag** (a throw-away WinForms window calling
`DoDragDrop` with a file list, steered by `SetCursorPos`, on the debug build
with its own APPDATA): `RegisterDragDrop` is `Ok`, a drag over the idle island
(190 x 34, clock showing) reaches `DragEnter` with `CF_HDROP`, the reducer
opens the panel on the shelf and a drop puts the card on it. So the chain
works when the pointer is *inside the drawn shape*.

**What was missing** (best-supported cause, the user's own drag is not
reproduced): the pointer was not on the island at all. A layered window with
per-pixel alpha only takes input on pixels it drew (alpha > 0), and the idle
Pill has an empty strip above it: `WindowFromPoint` at y = 2 is the Windows
search box, not Vale, so a drag aimed at "the top of the screen" never
entered. Fix: `HALO_ALPHA` (0.012, 8-bit alpha 3: invisible) is drawn on a
rectangle from y = 0 to 8 px under the island and 14 px beyond each side, for
every non-panel view. The margin takes a drag but not the pointer: moves,
dwell (`on_mouse_hover` disarms) and wheel over it are ignored
(`hit_at(..).is_none()`), so it opens nothing and scrolls nothing.
Side effect: clicks on whatever lies under that thin strip do not get
through while the island is small.

- OLE reports `Event::ShelfDragOver(bool)`; `App::on_shelf_drag` lights the
  drop zone (`ShelfState::dragging`, `Frame::shelf_drag`), passes `true` on as
  `ShelfDrag(true)` and, on leave, arms `TIMER_SHELF_HOLD` for 600 ms before
  `ShelfDrag(false)` (a drag that strays off the edge and back keeps the
  panel; re-entering cancels the timer).
- The reducer no longer ignores a drag during `fullscreen`: the island is
  hidden then, so no drag reaches it, and a wrong "fullscreen" reading (the
  shell's `QUNS_BUSY`) must not swallow a real one.
- Page: title with the count ("Rak berkas · 6"), a quiet "Kosongkan", cards
  with a 26 px icon over a name in up to two lines (`shelf::wrap_name`, one
  line with three rows), the X on a round chip, a "hilang" tag on a card whose
  file is gone, an empty state (dashed zone, download icon, hint), and the
  dashed accent drop zone with "Jatuhkan di sini" while a drag hovers. Demo
  scenes `shelf`, `shelf-empty`, `shelf-drop`, `shelf-drop-files`.

**Still not verified:** the halo with a real Explorer drag (the harness aimed
at y = 14 and at the idle island only; the y = 2 run was not completed), drops
on the halo itself, drag-out, and what the thin strip does to clicks on
windows behind it. `DI_SHELF_LOG` debugging was removed again.

Related: [[mixer]], [[open-mode]].
