---
tags: [gotcha, cli, focus, win32, ime]
date: 2026-10-27
---

# CLI panel: when the island may take the keyboard

Run 7 of the v3 Studio plan moved the CLI into the panel's CLI page and
deleted the separate CLI window. The island now owns keyboard input for that
page, which it never did before. Same family of gotchas as
[[studio-editor-frame]] (EDIT focus, borrows during `SetForegroundWindow`).

- **Only a click on the terminal activates the island.** `Hit::CliInput` →
  `Effect::FocusCliInput` → `App::focus_cli_input`: remember
  `GetForegroundWindow()`, `SetForegroundWindow(island)` (allowed: this
  process got the click), `SetFocus` on the hidden EDIT, then
  `Event::CliFocus(true)`. Never on hover, wheel, timers, notices or output.
  A fullscreen app hides the island, so it cannot be clicked there; there is
  no other path to activation. An open editor wins (`focus_open_editor`).
- **`MA_NOACTIVATE` and `WS_EX_NOACTIVATE` stay.** Every other click must
  leave the user's app active; activation is always explicit.
- **Letting go.** Esc, a page change, the panel collapsing (both from the
  reducer as `Effect::ReleaseCliFocus`) hand the foreground back to the
  remembered window, but only while the island still is the foreground and
  that window `IsWindow` and is visible. Otherwise `SetFocus(None)` so keys
  stop reaching the input. A click elsewhere or Alt+Tab arrives as
  `WM_ACTIVATE(WA_INACTIVE)` or the EDIT's `WM_KILLFOCUS` (`Key::Blur`): the
  new window is the target, nothing is handed back. `TIMER_SYSTEM` drops a
  focus flag that no longer matches `GetFocus()`/`GetForegroundWindow()`.
- **Reentrancy.** `SetForegroundWindow(prev)` sends `WM_ACTIVATE` and
  `WM_KILLFOCUS` synchronously while `APP` is borrowed: they queue in
  `DEFERRED_CLI` and run after the handler; by then the state is already
  released, so they are no-ops.
- **Layered windows do not paint children.** The EDIT is a 1 × 1 child at
  (0, 0) (a transparent pixel), never visible. The panel draws its text,
  selection and caret in DM Mono from `EM_GETSEL` and `GetWindowText`. Key
  and text changes post `WM_APP_CLI_REDRAW` once until handled.
- **IME.** The EDIT places the composition window at its own hidden caret;
  the subclass re-places it (`ImmSetCompositionWindow`, `CFS_POINT`) at the
  caret the renderer drew (`Renderer::cli_caret`, client px = EDIT client px
  because the EDIT sits at the origin). Not checked with a real CJK IME yet.
- **Tab** reaches the EDIT only with `WM_GETDLGCODE` → `DLGC_WANTALLKEYS`
  (and its `WM_CHAR` 0x09 is swallowed, as are Enter, Esc, Ctrl+A and a
  Ctrl+C without a selection, so nothing beeps).
- **Enlarged panel.** `ShapeKind::PanelTall` (560 × 440) needs a taller
  canvas: `IslandWindow::set_tall` switches to `TALL_WINDOW_H` (460) before
  the morph starts and back after it settles; `UpdateLayeredWindow` applies
  the size, so no `SetWindowPos`.

- **Folder menu.** Drawn inside the panel; it never takes focus or the
  foreground (no `SetFocus`, `SetForegroundWindow` or Windows menu), and
  the reducer emits no `FocusCliInput` / `ReleaseCliFocus` for its clicks.
  Esc with the input focused closes the menu first; the next Esc lets go
  as above. See [[cli-explorer-folder]].

- **Robot.** A running CLI command shows Bata's Think face, a finished one
  Happy or Sad for 4 s, see [[bata-robot-expressions]].

- **A second input: the Clipboard search.** `CliInput::create_with` makes
  another hidden 1 × 1 EDIT (id 202, 256 chars); each EDIT carries its own
  key callback in its subclass data, and the IME position is kept per EDIT,
  so the CLI's behaviour is unchanged. The same rules hold: only a click on
  the search field (`Hit::ClipSearch` → `Effect::FocusClipSearch`) activates
  the island; `ClipFocus` holds the panel; a page change, the panel
  collapsing, Blur or Alt+Tab let go (`Effect::ReleaseClipFocus`, the same
  `cli_prev` / `return_keyboard`). Typing filters, ↑↓ move the selection,
  Enter copies it, closes the panel (`Event::ClipCommit`) and gives the
  foreground back; Esc clears the text first and lets go on the next. The
  global hotkey opens the page without activating. See [[clipboard-history]].

- **A fourth input: the Notifikasi page** (id 204, 8 characters, typed text is
  dropped at once): only a click on the list's background or the title
  (`Hit::NotifFocus` -> `Effect::FocusNotifInput`) activates the island;
  `NotifFocus` holds the panel; a page change, a collapse, Blur or Alt+Tab let
  go (`Effect::ReleaseNotifFocus`). It is the one input that sends
  `Key::Delete` (the others keep it for editing). See [[notification-history]].

Related: [[studio-font-loading]], [[studio-icons]]

Part of [[v3-studio-overview]]. Listed in [[index]].
