---
tags: [gotcha, screenshot, editor, direct2d]
date: 2026-10-30
---

# Screenshot editor: paint order

Direct2D paints in call order, so a region background filled after the
buttons that live in it hides them. The editor's `Discard changes?` bar puts
its Discard and Cancel buttons inside the footer; the footer background used to
be filled after the buttons loop, so only the question text was visible while
every other button was dimmed.

- `draw` in `src/shot_edit/window.rs` follows the `PAINT_ORDER` const
  (`Canvas`, `ToolbarBack`, `FooterBack`, `Buttons`, `FooterText`). A unit
  test checks that `FooterBack` and `ToolbarBack` come before `Buttons` and
  `FooterText` after `FooterBack`.
- In bar mode the question's text box ends 12 px (scaled) before the Discard
  button.
- Keys while the bar is up: `confirm_key` (Enter = Discard, Esc = Keep), the
  rest ignored.

See [[screenshot-snap]].
