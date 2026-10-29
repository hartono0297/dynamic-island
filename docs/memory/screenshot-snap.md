---
tags: [decision, screenshot, capture, editor, uia, power, privacy]
date: 2026-10-29
---

# Vale's own screenshot: snap capture and editor

The Pintasan Screenshot tile runs Vale's own capture (`src/capture/`) and then
the editor (`src/shot_edit/`) unless Settings → Clipboard → *Alat screenshot*
is *Snipping Tool* (`ShotTool::Windows`, then `ms-screenclip:` as before). The
choice is made in `App::queue_shortcut`; `Action::Capture` waits `ANIM_MS + 60`
like the screen clip, so the collapsing panel is not in the picture. Config
`screenshot.tool` reads leniently: missing, a bad name or any non-string is
Vale.

- **Frozen frame.** `win::grab_virtual_screen` (GetDC + BitBlt
  `SRCCOPY|CAPTUREBLT` + GetDIBits, 32 bpp top-down) in physical px. It lives
  in memory only and is freed when the overlay closes; the crop is freed when
  the editor closes. Nothing touches disk or clipboard without Save or Copy.
- **Display affinity.** Every visible window of our own process (island,
  halo, stay-behind helper, an open editor) gets `WDA_EXCLUDEFROMCAPTURE` just
  for the BitBlt and its previous affinity back right after (`DwmFlush` in
  between). A window that refuses is hidden and shown again. Windows of our
  process are never snap targets.
- **Snapping** (`capture::pick`, pure): the frontmost top-level window under
  the pointer (lowest z, depth 0), then the smallest candidate of that window
  that holds the pointer, skipping boxes under 12 × 12. Elements of windows
  behind it never win. Click = press and release within 4 px (`is_click`);
  further is a drag (`drag_rect`, clamped to the virtual screen).
- **UI Automation budget** (`capture::uia`). One short-lived worker (COM MTA),
  never waited for: the overlay snaps to windows at once, finer boxes join
  when a window's result arrives (`WM_APP_UIA`, a boxed `Vec<Candidate>`,
  freed by the overlay even for a stale capture or at destroy). Frontmost 6
  windows, 400 boxes each (`FindAllBuildCache`, only BoundingRectangle cached,
  off-screen filtered out), 600 ms in all: `CUIAutomation8` as
  `IUIAutomation2`, connection and transaction timeouts set to what is left
  of the budget before each window, so a heavy or hung window cannot hold the
  worker past it. No timer, no thread at idle.
- **Windows list.** `EnumWindows` in z-order; skipped: invisible, minimised,
  cloaked, zero-size, `WS_EX_TRANSPARENT`, Progman/WorkerW (the desktop would
  swallow every click), own process. Rect from `DWMWA_EXTENDED_FRAME_BOUNDS`.
- **Overlay.** One `WS_POPUP` topmost over the virtual screen; Direct2D at
  96 dpi (a unit is a pixel); redraws only when the lit rect or the monitor
  under the pointer changes. One at a time (a second trigger is ignored).
  Display change cancels. If it cannot take the foreground (plain
  `SetForegroundWindow`, then a brief `AttachThreadInput` to the foreground
  thread) it cancels, so Esc/Enter never reach the app underneath.
- **Draw errors** (overlay and editor): only `D2DERR_RECREATE_TARGET` is
  retried, at most 3 in a row; anything else stops (overlay cancels; the
  editor shows the canvas without a picture too big for the device and says
  so in the footer) instead of repainting in a loop.
- **Editor.** Annotations in image px (`shot_edit::Session`: whole-document
  snapshots for undo/redo, `dirty` = anything changed since the last save or
  copy). `paint::draw_annotations` is used by the canvas and the export alike
  (D2D on a WIC bitmap), so a saved file equals what was seen. Text is a
  native EDIT (IME, paste); its subclass posts commit/cancel to the window so
  the EDIT is never destroyed inside its own proc. The post carries the
  EDIT's HWND and one from an EDIT that is no longer open is ignored; the
  subclass is removed before `DestroyWindow` (its WM_KILLFOCUS on the way out
  would otherwise close the next text box). Closing while dirty shows an
  inline bar, never a system dialog.
- **Save** runs on a worker (inbox + a doorbell message, so a closed window
  leaks nothing). It writes a snapshot; `Session::revision` is taken with it
  and `mark_clean_at` only cleans when nothing changed since. PNG encoding and `save_name` are the preview's own
  (`screenshot::win::encode_png`, `write_at`; create-new, never overwrites). A
  blocked write says "mungkin diblokir Controlled folder access" (see the
  gotcha in [[screenshot-preview]]).
- **Own-copy skip.** Copy writes PNG + CF_DIB through the clipboard module and
  records the sequence number (`shot_edit::record_own_copy`);
  `App::shot_from_image` returns early for that one image
  (`is_own_copy(seq, own)`), so the notch does not preview Vale's own copy.
  The clipboard history may still record it.
- **Demos** (debug builds, in `opens_windows`): `--demo-snap` (a desktop drawn
  in code, fixed candidates, no grab, no EnumWindows, no UIA) and
  `--demo-shot-editor [empty]`. In demos Copy is inert (the footer says so) and
  Save writes to `target/demo-shots/`; the process ends when the window closes.
  Screenshots of them: `PrintWindow` with flag 2 on the demo's main window.

Related: [[screenshot-preview]], [[clipboard-privacy]], [[studio-editor-frame]],
[[media-click-guards]], [[bata-idle-power-budget]]

Listed in [[index]].
