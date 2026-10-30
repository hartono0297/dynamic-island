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
