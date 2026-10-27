---
tags: [decision, settings, layout]
date: 2026-10-27
---

# Settings: Tampilan scrolls

The Appearance page was full (preview + 5 rows end at y 452 of the 496
margin in a 520 px client). Bata's four rows (Asisten di notch, Animasi,
Tidur setelah, Sapaan pagi) go in a second card under it, so the section
scrolls instead of growing the window.

- `settings_layout::layout` lays the section out unscrolled; `scroll()`
  then shifts its content (not the sidebar or title) by `State::scroll`,
  clamped to `scroll_max` (what runs past the bottom margin), and sets a
  `viewport` from `VIEW_TOP` (60) to the window's bottom.
- `Layout::hits` cuts every control's rect to the viewport, so a control
  scrolled under the title or below the window takes no click.
- The painter clips the content to the viewport and draws a thin
  scrollbar thumb (`Layout::scrollbar`).
- `WM_MOUSEWHEEL` scrolls one row (52 px) per notch (`scroll_by`); a
  section change resets it. Only Appearance has a viewport; the tests
  assert that, and that at the end the last card meets the margin.

The robot rows fade and take no clicks when the robot is off (`enabled`
on `Toggle` / `Segmented`, the stepper's existing flag); the sleep stepper
is also off for Mati. The design's "Hemat: tanpa kedip dan gerak, ekspresi
tetap" is too wide for one line beside the three segments with the
layout's width estimate, so it wraps (`Builder::wrap`, `row_text_lines`)
and the Animasi row grows by `DESC_LINE_H` per extra line
(`wrapped_row_h`). Do not shorten design copy to fit; wrap it.

Related: [[bata-robot-expressions]], [[bata-idle-power-budget]]

Part of [[v3-studio-overview]]. Listed in [[index]].
