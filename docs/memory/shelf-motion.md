---
tags: [decision, shelf, animation, power, v3.5]
date: 2026-10-30
---

# Shelf motion (v3.5)

Cards on the [[shelf]] page animate in, out and across. Pure state in
`src/ui/shelf_motion.rs` (tested), the app side in `src/app_shelf.rs`, the
drawing in `paint_shelf` / `paint_shelf_card` (`render_panel.rs`). It follows
the `FrameClock` and `animating_at` pattern of [[panel-linger-and-motion]] and
the budget of [[bata-idle-power-budget]]: no timer.

- **Enter** (`SHELF_ENTER_MS` 200, ease-out cubic): opacity 0 to 1, scale
  0.92 to 1 around the card centre, a 6 px rise. Files dropped together
  stagger 30 ms apart, capped at 12 cards.
- **Exit** (`SHELF_EXIT_MS` 180, ease-in-out cubic): scale 1 to 0.85 and
  opacity 1 to 0 as a **ghost** at the card's old rect. The ghost is dropped
  only when its exit ends. A card removed mid-entrance leaves from the alpha
  and scale it had reached. "Kosongkan" exits every card at once.
- **Move** (`SHELF_MOVE_MS` 220, ease-out cubic): lerp of x, y, w, h from the
  old rect to the new one (after a removal, or when rows or name lines
  change). A change mid-way starts from the rect drawn at that moment
  (`card_at` of the old rect), so nothing jumps.
- **Header count and empty hint** wait for the ghosts: the count is the final
  cards plus the ghosts still showing, and the empty state does not draw
  under them. "Kosongkan" itself follows the final list, so it goes at once.

## How it is wired

- Cards are told apart by `ShelfCard::key`, the normalised path (`key_of`:
  lower case, no trailing slash). `ShelfMotion::on_change(old_cards,
  old_rects, new_cards, new_rects, now, enabled)` diffs by key. Rects come
  from `panel_layout::shelf_layout(shape_for(style, Panel), strip, n)`.
  `shelf_card_layout(rect, lines)` was split out of `shelf_layout` so a card
  between two places gets its parts (icon, label, X, "hilang") from the rect
  it is drawn at, with one formula.
- `app_shelf.rs` snapshots the cards before the list changes (`on_shelf_drop`,
  `shelf_remove`, `shelf_clear`) and calls `shelf_animate`, which starts the
  clock with `ensure_clock` when something runs. A drop onto a panel that is
  not on the shelf yet holds the entrance back 120 ms (`OPEN_DELAY_MS`), so
  the panel's own opening does not hide it.
- `App::animating_at` includes `shelf.motion.running(now)`; `animate` calls
  `prune(now)` after the frame is drawn, so when the last animation ends the
  state is empty and the clock stops. Idle cost stays zero.
- `sync_shelf` clears the motion when the page is not on screen, and with
  Windows' animation effects off nothing is recorded (`App::animations`,
  `client_animations_on`): changes are instant.
- The motion reaches the renderer as `Frame::shelf_motion`; `Frame::shelf`
  stays the final list, and hit regions use the final layout. A ghost takes no
  click, a card still entering or sliding takes clicks at its final rect, and
  drag-out reads the real list (`self.shelf.cards`).

## Gotchas

- **No layer for the fade.** `paint_page_at` already holds `layer2` during a
  page cross-fade or a reveal, and D2D cannot push one layer twice
  (`EndDraw` would fail). The card alpha is applied per draw (every fill, icon
  and text takes an alpha), and the scale is a `SetTransform` composed on the
  page's translation (`GetTransform`, scale around the centre, restored after).
  Translucent parts of one card overlap (the X chip over the card), which is
  not visible at 200 ms.
- **Paint order**: ghosts first, live cards over them, the drop zone's dashed
  outline last (see [[shot-editor-paint-order]] for why order matters in D2D).
- **Regions after the last frame**: the shelf never holds the regions back
  (they come from the final layout), so the frame where the last animation
  ends registers them like any other (compare the cross-fade gotcha in
  [[panel-linger-and-motion]]).
- A sticky strip appearing during an animation shifts the final layout a few
  px; ghosts keep the rect they were drawn at. Accepted.

## Demo

Scenes `shelf-enter` (3 cards, 3 more dropped), `shelf-exit` (second of six
taken off) and `shelf-move` (third of nine, three rows become two), made-up
names only. `DI_DEMO_SHELF=0..1` freezes the progress of the scene's span
(documented in the `demo.rs` module docs).

**Not verified live:** the drawing was only run in `--demo` to check that
nothing errors (no frame was compared by eye), a real drop from Explorer, the
120 ms delay against the panel's opening, a removal with the real click, and
Windows' animation effects switched off with the shelf open.
