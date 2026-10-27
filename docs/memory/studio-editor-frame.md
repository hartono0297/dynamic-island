---
tags: [decision, editor, direct2d, gotcha]
date: 2026-10-27
---

# Editor frame: frameless DWM window, Direct2D, borderless EDITs

Run 5 of the v3 Studio plan replaced the Win32 dialog look of the editors
(`src/ui/editor.rs`). The event and reminder editors both use it. What to
know before touching it:

- **Frame.** `WS_POPUP` with no caption, `WS_EX_TOOLWINDOW | WS_EX_TOPMOST`,
  owned by the island. Windows 11 rounds it via
  `DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND`; `DWMWA_BORDER_COLOR` is the
  palette hairline blended over the fill (`set_frame`). `CS_DROPSHADOW` on the
  class. The header drags through `WM_NCHITTEST` → `HTCAPTION`, except the
  close button (`Layout::is_caption`).
- **Drawing.** Everything but text input is Direct2D on a software
  `ID2D1HwndRenderTarget`, opaque, **grayscale** text antialiasing (no
  ClearType fringes, the old "teks pelangi"). `editor_paint::Gfx` is shared
  by the editor and its popover (`Rc`); each window keeps its own `Target`.
  Toggle and segment track come from `settings_controls` (`draw_toggle`,
  `draw_segment_track`, `Pen`).
- **EDITs** stay real controls (IME, paste, undo): no border, colours from
  `WM_CTLCOLOREDIT` (title on `bg`, notes on `field`), GDI font with
  `ANTIALIASED_QUALITY`. Placeholders are painted after the EDIT's own
  `WM_PAINT` in its subclass, with the caret hidden meanwhile
  (`EM_SETCUEBANNER` does not work on multi-line EDITs).
- **Gotcha: focus on activation.** `DefWindowProc(WM_ACTIVATE)` focuses the
  frame itself before `focus_last` runs, so `WM_SETFOCUS` on the frame must
  not be recorded as the last focus, or typing right after opening is lost.
  Record `LAST_FOCUS = frame` only when a drawn item is focused on purpose.
- **Gotcha: resizing inside a borrow.** `SetWindowPos` sends `WM_SIZE`
  synchronously; `refresh()` computes the layout inside `with_editor`, then
  resizes and moves the EDITs after releasing it.
- **Popovers** (`editor_popover.rs`): owned frameless popups, same corners.
  They take the keyboard. A pick is posted (`WM_APP_POPOVER_PICK`), the end
  too (`WM_APP_POPOVER_CLOSED` with refocus/keyboard/save flags). Losing
  activation closes the popover only, never the editor. The click that
  closed a popover by deactivation must not reopen it: the chip checks
  `popover::current()` (still closing) and a 300 ms `dismissed` guard.
- **Month grid** is shared with the panel: `ui/month_grid.rs`
  (`month_cells`, `day_geometry`, `day_marks`, `paint_day` through
  `CellCanvas`). The panel fills today and rings the selection
  (`Filled::Today`); the picker fills the pick and rings today
  (`Filled::Selected`).
- **New rows** go in `editor_layout`, with tests, the way the reminder's
  quick-time pills and weekday chips did (see [[studio-reminder-editor]]);
  a row that appears or hides grows the window through `refresh`. Dates
  never come from the Windows locale.
- Demo: `--demo-editor event|event-bad|event-allday|...` with
  `DI_POPOVER=calendar|time|start`, `DI_WINDOW`, `DI_LANG`. Screenshots taken
  from a non-DPI-aware PowerShell are scaled; call `SetProcessDPIAware` first.

Related: [[studio-font-loading]], [[studio-tabular-figures-word-gaps]],
[[studio-light-secondary-text-contrast]], [[studio-calendar-day-view]],
[[cli-panel-keyboard-focus]]

Part of [[v3-studio-overview]]. Listed in [[index]].
