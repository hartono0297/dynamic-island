---
tags: [decision, robot, power, timers]
date: 2026-10-27
---

# Bata's idle power budget

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

Gotcha: the System page and the clipboard were built in parallel runs and
both took ids 17 and 18; the merge moved the clipboard's to 20–22.
`app::tests::timer_ids_never_collide` now checks every id.

Related: [[bata-robot-expressions]], [[studio-media-progress-timer]], [[system-page]],
[[clipboard-history]],
[[lyric-notch-width]] (a long lyric line's one-shot scroll timer follows the same rule)

Part of [[v3-studio-overview]]. Listed in [[index]].
