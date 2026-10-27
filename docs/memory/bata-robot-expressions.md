---
tags: [decision, robot, notch]
date: 2026-10-27
---

# Bata: which face, and where it shows

`src/ui/robot.rs` draws Bata from `robot()` in
`docs/design/studio-robot.html` (64-unit grid, fixed brand colours
#E2683C / #17120F / #F4EDE4, never the accent). `choose_expr` picks the
face; its priority, highest first:

1. Panel or hidden: no robot. The panel stays clean.
2. A notice decides by kind: reminder, Claude waiting → Alert; pomodoro
   phase done, Claude done → Happy; low battery → Sad; greeting → Wave.
   `NoticeKind::App` → no robot: other apps keep their badge. The island's
   own messages (config invalid, DND hint) also use `App { app_id: None }`
   and so keep their badge too; they cannot be told apart.
3. Pointer over a compact island → Happy (+ 2 px lift), before the dwell
   opens the panel. It also wakes a sleeping robot. The lift only applies
   where the hover sets the face (`Spot::compact`): a notice keeps its face
   still, and no frame clock runs for a robot that is not drawn.
4. Pomodoro → Focus, media → Music (the robot replaces the album art),
   lyric line → Sing.
5. Idle only: CLI just done/failed (4 s) → Happy/Sad; Claude working or a
   CLI command running → Think; battery at or below the threshold and
   discharging → Sad; asleep → Sleep; else Idle.

"Claude working" = a session with `ClaudeActivity` in the last 2 minutes
that has not sent Done or Waiting since (`CLAUDE_WORKING_FOR`).

Geometry: compact shapes widen by `robot_extra` (media +2, lyric +20,
pomodoro +26; idle holds robot + clock centred as a group). Notices are
unchanged: the 40 px robot sits exactly in the badge's square. With the
robot off every width is as before (`shape_with_robot(.., false)`).

Blink only shows on open eyes (Idle, Focus, Alert, Think, Sad); the
design's `blink` eyes, fixed at (L, Y + 1) and (R, Y + 1) whatever face
blinks, replace the first two marks.

Related: [[bata-idle-power-budget]], [[bata-morning-greeting]],
[[studio-bata-logo-sources]]

Listed in [[index]].
