---
tags: [decision, robot, pointer, motion, power]
date: 2026-10-30
---

# The idle cat looks toward the pointer

The cat in the idle notch turns its head toward the pointer, sideways only, with
an eased move. It shares one poll with the notch peek ([[notch-peek-eyes]]);
the cost rules are in [[bata-idle-power-budget]], the faces in
[[bata-robot-expressions]].

## When

`App::look_eligible`: `config.robot` on, `motion().moves()` (Full, and not
Energy Saver), the view is **Idle**, the cat is drawn there (`robot_frame` is
Some), the peek is not shrunk, not fullscreen, not `screen_away`. Idle only
(assumption): in the compact music, lyric and timer views the cat's slot is
tight, so it stays still there. Any face in Idle follows, a sleeping one too.

## Maths (`pointer_watch.rs`, tested)

- `look_target(pointer_x, notch_centre_x, monitor_left, monitor_right)` in
  [-1, 1]: the horizontal offset over the distance from the notch centre to
  that side's monitor edge, clamped. Pointers on another monitor clamp to +-1.
  The island sits at the primary monitor's centre.
- `LookEase { current, target }`: `set_target` ignores a change under
  `LOOK_EPS` (0.02) and returns false (no redraw, no clock); `advance(dt)` is an
  exponential ease with tau 120 ms (`1 - e^(-dt/tau)`, the same at any frame
  rate, never overshoots), snapping within 0.002; `release()` sends the target
  back to 0 whatever the distance; `snap_to_centre()`.
- `Pose::looking(v)` (`ui/robot.rs`): `head_dx_dip = 8 v`, `tilt_deg = 6 v`,
  `eye_dx_dip = 4 v` (constants `LOOK_HEAD_DIP`, `LOOK_TILT_DEG`,
  `LOOK_EYE_DIP`). `RobotPainter::paint` slides and tilts the head and ears
  (the tilt adds to the face's own, around the same pivot (32, 40)) and slides
  the face's marks by the extra eye offset, all converted with `k = area.w /
  GRID`.

## The clamp

No reduction was needed: with the cat at +-8 dip, 6 degrees and the eyes 4 dip
further, the head, the ears' strokes and the eyes stay inside the idle island
and clear of the clock box for both styles and every notch size
(160..300 x 28..44), with or without the clock
(`the_looking_cat_stays_in_the_idle_island`, geometry.rs, applies the pose's
transform to the head outline, the ear tips and the ears' outer strokes). The
tightest case: the head's right edge reaches about 32.6 dip from the cat slot's
left edge against the clock box at 34. If the idle layout changes (a wider
clock, a smaller gap), that test says whether `LOOK_HEAD_DIP` must come down.

## Cost

The poll (`TIMER_POINTER`) is slow far from the top edge (250 ms) and 33 ms
within 150 dip of it. The look itself only runs the `FrameClock` while the ease
moves: a poll that changes the target by 0.02 or more calls `ensure_clock`;
`animate` eases the look before each draw (`advance_look`); `animating_at`
counts `look.moving()`. When the cat stops being eligible the target goes to 0
(eased with Full motion while the cat is still on screen, snapped otherwise;
in practice it is almost always snapped, since a cat that cannot follow is
usually not on screen).

Demo scenes: `bata-look-left`, `bata-look-right` (the idle cat at -1 / +1).

Not verified live: the feel of the ease with a real pointer, and how the tilt
reads on a see-through notch.
