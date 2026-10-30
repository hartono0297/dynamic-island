---
tags: [decision, panel, settings, v3.4]
date: 2026-10-29
---

# Panel pages: order and which ones the wheel shows

Run 9 of the v3.1 plan. Settings → Tampilan → "Halaman panel" lists all 12
pages (every page in `Page::ALL`) in the owner's order, each with an order
stepper (− earlier, + later) and a toggle. `src/pages.rs` (`PanelPages`) is the pure model.

- **Every page is always listed once.** `config.json` keeps `panel_pages` as
  `[{page, on}]`; unknown and repeated names are dropped, a page the file
  does not name (an older config) joins right after its default neighbour,
  and a list with every page off shows them all. A bad value never throws the
  rest of the file away (lenient deserializer).
- **Off means skipped, not gone.** The wheel and the page dots only count the
  pages that are on, but a page's own button still opens it (the Clipboard
  tile, a disk warning's "Buka"); on such a page no dot is lit and the wheel
  moves to the nearest page that is on.
- **The last page shown cannot be switched off**: its toggle is disabled.
- **A page switched off while it shows** hands over to the next page on (or
  the one before it at the end of the order).
- **Slides follow the owner's order**, and the page drawn beside the new one
  is the page it came from (`Frame::page_from`), not a neighbour in
  `Page::ALL`.
- The card sits at the end of Tampilan, below Bata's card.

- A page that is off but opened by its button holds only for that visit:
  when the panel closes the page moves to the nearest one on, so the next
  hover does not land on it.

Related: [[settings-appearance-scroll]], [[v3-studio-overview]]

**v3.5:** `Page::Devices` sits between System and CLI, so `Page::ALL` is 12
pages ([[devices]]); a saved list without it gets it back right after System
(`from_saved`, tested).

## The Settings card, remade (2026-10-29)

The "− n +" order stepper was confusing and is gone. Settings → Tampilan →
Halaman panel now reads:

- **Shown pages first**, in order: a drag handle at the left, up/down
  buttons (`PanelPages::move_shown`: among the pages shown, stepping over
  hidden ones) and *Sembunyikan*. Under a small "Tersembunyi" title come the
  hidden pages, each with *Tampilkan*. The header says "Tampil di panel:
  n/12" and holds *Kembalikan bawaan* (`PanelPages::default()`; off while
  the list already is the default).
- **The last page shown cannot be hidden:** its button is disabled and the
  row says "Minimal satu halaman tampil".
- **`PanelPages::move_to(page, index)`** takes a page out and puts it at a
  place of the whole order (hidden pages count); a drag drop maps "place
  among the shown" to the entry index of the page it lands on
  (`settings::page_drop`). `move_by` is gone.
- The drag, the raised row and Esc to cancel are shared with the Pintasan
  card, see [[shortcut-tiles]]. A Shortcuts tile can now open any page,
  including a hidden one (for that visit only, as the Clipboard tile).

A sideways two-finger swipe over the open panel follows the same order and
skips as the vertical wheel (`Event::PageFlick`, see [[gestures]]).

## Page dots become a dock (hover)

Hovering the page dots under the open panel turns them into a row of page
icons with macOS-Dock magnification. Pure model in `src/ui/dock.rs` (tested),
the pointer and the frame step in `src/app_dock.rs`, drawing in
`render_panel::paint_page_dots`, the demo in `dock-hover` and
`dock-hover-edge` (`DI_DOCK_X=<dip>` moves the pointer).

- **Zone.** The dots' bounds grown by 10 dip vertically and 8 horizontally.
  While expanded, the zone is the resting icon row's bounds plus the same
  padding (hysteresis, so the growing icons do not drop the pointer). Leaving
  the zone, `WM_MOUSELEAVE`, or a page with no dots (cleaner, Mole, search:
  the renderer registers no `DockZone`) eases the row back to dots; the panel
  closing resets it at once (`App::dock_sync`, before every draw).
- **Resting layout** (`resting_layout`, never moves): `DOCK_ICON` 14 dip at a
  `DOCK_PITCH` of 28 dip, centred like the dots, bottom edge on the dots'
  line (`s.y + s.h - DOT_BOTTOM`). One icon per page of `frame.pages` (the
  owner's order, `PanelPages::visible`), art from `shortcuts::page_icon`; the
  active page in the accent colour, the others secondary. A backdrop pill in
  the surface colour sits behind the row so icons growing over the content
  stay legible. Falloff and click rects read this layout only, so an icon
  growing under the pointer never shifts its own target (no shake).
- **Falloff.** Target scale `1 + 0.9 * falloff(|pointer_x - centre|)`,
  `falloff(d) = 0.5 * (1 + cos(pi d / 56))` below 56 dip, else 0. Peak 1.9x,
  the icon grows from its bottom edge. Pitch 28 with reach 56 keeps
  neighbours from overlapping (tested).
- **Smoothing.** Mouse moves only set targets. The shown values (each scale
  and the expand progress `e`) advance once per `FrameClock` frame by
  `v += (target - v) * (1 - exp(-dt / 0.07))`: frame-rate independent, no
  overshoot, snaps within 0.002. During the morph each dot lerps to its
  resting icon by `e` and the two crossfade. With Windows animations off
  (`App.animations`) the values snap.
- **Click.** `Hit::PanelPage(page)`, active only while `e > 0.5`. Each icon
  owns a pitch-wide cell from the baseline up to the grown height (so what
  is seen is what is hit; hit-testing never reads the scales). The current
  page takes no region. In `state.rs` `go_to_page` follows the flick's rules
  (vertical residue zeroed, the sticky-reminder peek spends the click,
  nothing on the search or a closed panel) and the linger arms as a page
  turn (`LingerTrigger::PageTurn`, so the current page arms nothing). The
  slide direction is the pages' places in the owner's order, as ever.
- **Power.** No timer: see [[bata-idle-power-budget]]. `App::animating_at`
  includes `dock.moving()`; a pointer resting on an icon stops the clock.

Related: [[panel-linger-and-motion]], [[gestures]]
