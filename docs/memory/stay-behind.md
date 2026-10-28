---
tags: [decision, window, experimental, v3.2.1]
date: 2026-10-29
---

# Stay behind windows (v3.2.1)

Settings → Umum → **Di belakang jendela** (`config.stay_behind`, default
**off**, island-owned like the other toggles). Experimental: it was left out of
3.2.0 on purpose because it can fight other windows for the order.

- Off: nothing changes, nothing runs. The island is topmost as always.
- On: the island sits at the **bottom** of the window order
  (`IslandWindow::set_front(false)` → `SetWindowPos(HWND_BOTTOM, NOMOVE |
  NOSIZE | NOACTIVATE)`, which also drops the topmost style), so ordinary
  windows cover it. It never takes focus.
- It comes up when (`src/behind.rs`, pure `Behind::decide`, tested):
  - the view must be seen (`view_needs_front`: anything but Idle, the music
    and timer compact views and Hidden: notices, panels, search, previews,
    shelf drags);
  - the pointer is on the 3 px strip at the screen's top edge over the
    *drawn* island plus 12 dip each side (`STRIP_PX`, `STRIP_MARGIN_DIP`,
    `on_strip`, `App::on_raise_strip`; not the 620 px window, which is wider);
  - the pointer is still over the drawn island while it is up.
  It goes back down 250 ms (`LOWER_AFTER`) after none of those holds.
- Cost: one `TIMER_BEHIND` (29) at 80 ms (`POLL_MS`) reading `GetCursorPos`,
  only while the setting is on and the screen is not fullscreen
  (`App::sync_behind`). Fullscreen or turning it off puts the island back on
  top and stops the poll. A view change raises it at once (`sync_timers`), not
  at the next poll.

**Limits (not verified live):**
- `HWND_BOTTOM` is the bottom of the *normal* order. Whether the island stays
  visible above the wallpaper and desktop icons, rather than under the desktop
  window, was not tried; if it vanished the strip still raises it
  (`HWND_TOPMOST`).
- A maximised window's title bar is at y = 0: the strip lies over it, so the
  island rises over that title bar while the pointer is there.
- A window covering the top-centre of the screen hides the island until the
  pointer touches the strip.
- Games and fullscreen apps: the existing fullscreen detection hides the
  island as before; the order is reset to top when fullscreen starts.

Related: [[open-mode]], [[v3-2-handoff]].
