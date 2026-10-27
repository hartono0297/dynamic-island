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

The clipboard history ([[clipboard-history]]) adds no repeating timer
either: the listener costs nothing until a copy; `TIMER_CLIP_SAVE`,
`TIMER_CLIP_RETRY` and `TIMER_CLIP_HOLD` are one-shots; a change while the
Clipboard page is hidden draws nothing unless a notice shows; "Disalin"
rides the notice deadline (1.5 s).

Related: [[bata-robot-expressions]], [[studio-media-progress-timer]],
[[lyric-notch-width]] (a long lyric line's one-shot scroll timer follows the same rule)

Part of [[v3-studio-overview]]. Listed in [[index]].
