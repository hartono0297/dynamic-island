---
tags: [decision, gotcha, panel, hover, notices, v3.2]
date: 2026-10-28
---

# Open mode and notices with buttons (v3.2 Run 1)

Settings → Umum → **Mode buka** (`config.open_mode`, `OpenMode`): `Hover` (the
default, as always: opens under the pointer, closes when it leaves),
`HoverStay` (opens on hover, stays until a click outside the island) and
`Click` (opens on a click on the compact island, stays until a click
outside). A typo in the file reads as `Hover`; the field is the island's own
(`merge_owned`).

## How it works (`state.rs`)

- `latched` is "kept open". `panel_open()` is `pointer || latched || editor ||
  CLI input || clipboard`. `pointer` is hover, unless the mode is Click.
  `collapse()`, a fullscreen app and a mode change clear the latch.
- `Event::Dismiss` (a button went down outside the shape) closes a latched
  panel. Under the pointer it stays shut until the pointer comes back
  (`collapse()`), so it never reopens at once.
- Click mode: `Hit::Body` on Idle, MediaCompact, MediaLyric and
  PomodoroCompact opens (a ready pomodoro starts from inside the panel).
  `hover_opened` starts the media guard, so a second click during the
  opening hits nothing ([[media-click-guards]]).
- What else holds the panel (an editor, the CLI input, the clipboard search)
  survives a click outside, as it did before.

## Outside clicks without a hook (`app.rs`)

`TIMER_OUTSIDE` (24, repeating 40 ms) runs **only while** `outside_click_wanted()`
(latched, open, not fullscreen). Each tick reads `GetAsyncKeyState` for the
left, right and middle buttons (so a swapped pair does not matter) and sends
`Dismiss` on an up-to-down edge outside `renderer.hit_test` (None means
outside the drawn shape). A press already down when the polling starts is not
an edge (`PressEdge::reset`). Rejected: `SetCapture` (swallows the click and
fights `TME_LEAVE`), a global low-level mouse hook (fires on every move and is
removed silently if the UI thread is slow), deactivation (the window never
activates). It never swallows a click, so the press-release pairing of
[[media-click-guards]] is untouched. Cost: 25 wakeups a second while a panel
is kept open, none otherwise; the note [[bata-idle-power-budget]] has the
timer id.

## Notices with a button

Root cause of "hover gets in the way of Selesai": `view()` checked
`panel_open()` before the sticky notice, so a 120 ms dwell opened the panel
and the reminder shrank into a small strip that moved during the opening.

- `notice_has_action`: Reminder, MoleDone, SystemLow, Pomodoro, Screenshot
  and an App notice with an app id. A hover that starts on one (or on a
  reminder that is the newest sticky notice) sets `hover_blocked`: the panel
  stays shut for that visit. A wheel notch over it peeks at the panel; a new
  visit clears it.
- While the pointer is on such a notice its expiry waits; once it leaves,
  `SHOT_LEAVE_GRACE` (2 s) first. A click that dismisses it does not pop the
  panel under the pointer, except SystemLow, MoleDone and MoleFailed, whose
  click leads to the panel.
- **Gotcha:** Claude waiting and a failed Mole run are sticky and only inform;
  they must NOT block hover, or the panel could not be opened while they
  show. Only the reminder (its Selesai button) blocks for a sticky notice.
- A notice that arrives over an open panel changes nothing: the panel stays
  and shows it as a strip.

Esc closing a kept panel is not global (a global Esc would close the island
from any other app); it stays inside the CLI and search inputs.
