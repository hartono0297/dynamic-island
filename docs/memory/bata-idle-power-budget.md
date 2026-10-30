---
tags: [decision, robot, power, timers]
date: 2026-10-29
---

# The notch assistant's idle power budget

An idle island must stay cheap: no continuous timer, no frame thread.
Bata adds only one-shots (`app.rs`, planned by `robot::plan_timers`):

- `TIMER_BLINK`: a random 4–7 s wait, then 120 ms with the eyes shut,
  then re-armed. Only while a robot with open eyes shows (compact view or
  the island's own notice), motion is Full, and nothing animates. A
  sleeping, happy or music face runs no blink timer.
- `TIMER_ROBOT`: one-shot at `Brain::next_change`: falling asleep
  (last activity + N min), a Claude session going quiet, a CLI face
  ending. Once asleep with nothing pending, no robot timer runs at all.
- Hover lift (140 ms) and the alert's two bounces (2 × 260 ms) run the
  existing `FrameClock` only while they last (`App::animating_at`), and
  the blink re-plans when the clock stops.
- The stay-behind rise ([[stay-behind]]: 180 ms, or a 120 ms fade with
  Motion not Full) runs the same `FrameClock` only while it lasts; no timer.
- Panel transitions ([[panel-linger-and-motion]]): the sub-view `Reveal`
  (160 ms), the search's page cross-fade and a page slide are counted in
  `animating_at` like the rise: the `FrameClock` starts when one begins and
  stops when none is left, no timer. The panel's **linger** (4 s after an
  action) adds no timer either: its deadline rides `next_deadline()` on the
  existing `TIMER_DEADLINE`; while it runs, the click-outside poll
  (`TIMER_OUTSIDE`, below) watches so a click outside ends it. Windows'
  animation setting is read at start and on `WM_SETTINGCHANGE`, never polled.

Motion policy (`Motion`, config `robot_animation`):

- **Penuh (Full)**: blink, lift, bounce.
- **Hemat (Saver)**: no blink, lift or bounce; faces and sleep still change.
- **Full with Windows Energy Saver on** (`SYSTEM_POWER_STATUS.SystemStatusFlag
  == 1`, read with the battery every 30 s and on `PBT_APMPOWERSTATUSCHANGE`)
  behaves as Saver.
- **Mati (Off)**: no robot timers at all. Faces still follow state, but
  nothing timed: no sleep, no 4 s CLI Happy/Sad, and a Claude session
  that dies without Done keeps Think until the next event. The sleep
  stepper is disabled in Settings for it.

Robot off: nothing of this runs; widths, timers and drawing are as before.

The System page adds two one-shots (see [[system-page]]):

- `TIMER_SYSTEM_PAGE` (17): 2 s, re-armed on each sample, and only while the
  panel shows the System page. It is killed when the page or panel closes.
- `TIMER_SYSTEM_WARN` (18): 5 min (sooner only to confirm RAM's minute),
  re-armed when its reading returns. It is the only new timer while the
  panel is closed. Its reading runs on the sampler thread, and the island
  redraws only if it raises a notice.

The clipboard history ([[clipboard-history]]) adds no repeating timer
either: the listener costs nothing until a copy; `TIMER_CLIP_RETRY` (20),
`TIMER_CLIP_SAVE` (21) and `TIMER_CLIP_HOLD` (22) are one-shots; a change
while the Clipboard page is hidden draws nothing unless a notice shows;
"Disalin" rides the notice deadline (1.5 s).

The screenshot preview ([[screenshot-preview]]) adds `TIMER_SHOT_RETRY`
(23), a one-shot that runs at most twice after a new clipboard image; its
folder looks run on short-lived threads, never a watcher.

## Pointer polls (v3.2.1 review, 2026-10-29)

A measured review found ~78 main-thread wake-ups a second at idle (v3.1.0:
~15); the 50 ms halo poll was ~55 of them. The three pointer polls now use
`SetCoalescableTimer` (slack `timer_slack`: half the interval, ≤ 30 ms), are
paced by pure helpers (tested), and all pause while the session is locked or
the display is off (`screen_away`: `WTS_SESSION_LOCK` and
`GUID_CONSOLE_DISPLAY_STATE` via `RegisterPowerSettingNotification`, both
calling `sync_pointer_polls`).

| Timer | Before | Now |
|---|---|---|
| `TIMER_HALO` (30) | 50 ms whenever the island was up, not a panel | none at idle; after a trigger ([[shelf]]): 250 ms, 50 ms only with the left button down within 300 dip; stops after 3 s near without a press, or at once away |
| `TIMER_OUTSIDE` (24) | 40 ms while a panel is kept open (can be hours) | 40 ms with input in the last second, else 120 ms (`outside_poll_ms`, `GetLastInputInfo`); a `WM_MOUSEMOVE` speeds it up at once |
| `TIMER_BEHIND` (29) | 80 ms while "Di belakang jendela" is on | 80 ms near the top edge (150 dip) or while lowering is due, else 350 ms (`behind::poll_ms`) |
| `TIMER_POINTER` (33) | new, 2026-10-30 | one `GetCursorPos` for the notch peek and the cat's look, see below |

So an idle island, pointer elsewhere, runs none of them except
`TIMER_POINTER` (with stay-behind on: ~3 a second). Not measured again after
the change.

### `TIMER_POINTER` (the notch peek and the cat's look)

One repeating timer through `App::pace` / `SetCoalescableTimer`, handled like
`TIMER_BEHIND`. Paces (`pointer_watch::poll_ms(follow_on, peek_on,
near_top)`; near = the pointer within `behind::NEAR_TOP_DIP`, 150 dip, of the
island monitor's top edge):

| Cat follows (look) | Peek | near | far |
|---|---|---|---|
| on | any | 33 ms | 250 ms |
| off | on | 80 ms | 350 ms (the same as `behind::POLL_MS` / `SLOW_MS`) |
| off | off | no timer | no timer |

It stops while fullscreen, while the session is locked or the display is off
(`screen_away`), and whenever neither feature is eligible: the peek needs
stay-behind off and a quiet compact view ([[notch-peek-eyes]]); the look needs
the cat on, motion Full (not Energy Saver) and the idle view
([[cat-looks-at-pointer]]). `App::sync_pointer_watch` starts, re-paces or
stops it, called from `sync_timers` (so every `refresh`), `sync_pointer_polls`
(lock, display), `follow_pointer_settings` (config adoption: stay_behind,
robot, robot_animation; Energy Saver goes through `check_battery`'s refresh;
fullscreen through the event's refresh). Each tick does one `GetCursorPos`
and feeds both. The window walk of the peek is not part of the poll's idle
cost: it runs only when the pointer first comes into the zone round the notch
and at most once a second while it stays.

**The look's ease runs on the `FrameClock`**, not on this timer: a poll that
moves the look target by 0.02 or more starts the clock (`ensure_clock`),
`animating_at` counts `look.moving()`, and the clock stops once the ease has
arrived. A small pointer movement costs no frame. The shrink to the eyes is a
normal morph on the same clock (or settles at once with Motion not Full); while
the notch is shrunk the blink timer and the equalizer timer are off.

Gotcha: the System page and the clipboard were built in parallel runs and
both took ids 17 and 18; the merge moved the clipboard's to 20–22.
`app::tests::timer_ids_never_collide` now checks every id.

Related: [[bata-robot-expressions]], [[studio-media-progress-timer]], [[system-page]],
[[clipboard-history]],
[[lyric-notch-width]] (a long lyric line's one-shot scroll timer follows the same rule)

Part of [[v3-studio-overview]]. Listed in [[index]].
