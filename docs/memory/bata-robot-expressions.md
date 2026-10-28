---
tags: [decision, robot, notch]
date: 2026-10-28
---

# The notch assistant: which face, and where it shows

Since v3.1 (Run 13) the assistant is Vale's cat, not the robot Bata.
`src/ui/robot.rs` draws it from `cat()` variant B in
`docs/design/vale-kucing-simpel.html` (64-unit grid, fixed brand colours
#E2683C head / #17120F eyes / #F4EDE4 shine and extras / #B07A9A bow, never
the accent): head, two ears, eyes with two lashes each, a plum bow at the
base of the right ear. No mouth, nose or whiskers. The cat has no name yet
(the owner picks one); the UI says "kucing Vale" and notices come from
"Vale". Code and demo steps still say `robot` / `bata` (no rename churn).
`choose_expr` picks the face (unchanged from Bata); its priority, highest
first:

1. Panel or hidden: no robot. The panel stays clean.
2. A notice decides by kind: reminder, Claude waiting → Alert; pomodoro
   phase done, Claude done, a finished clean ([[cleaner]], or a Mole one, [[mole-mode]]),
   "Disalin" (`NoticeKind::Copied`, see [[clipboard-history]]) → Happy;
   low battery, low disk/RAM, a failed Mole clean → Sad; greeting → Wave.
   With the robot off, "Disalin" shows the Clipboard icon badge.
   `NoticeKind::App` → no robot: other apps keep their badge. The island's
   own messages (config invalid, DND hint) also use `App { app_id: None }`
   and so keep their badge too; they cannot be told apart.
3. Pointer over a compact island → Happy (+ 2 px lift), before the dwell
   opens the panel. It also wakes a sleeping robot. The lift only applies
   where the hover sets the face (`Spot::compact`): a notice keeps its face
   still, and no frame clock runs for a robot that is not drawn.
4. Pomodoro → Focus, media → Music (the robot replaces the album art),
   lyric line → Sing.
5. Idle only: CLI just done/failed (4 s) → Happy/Sad; Claude working, a
   CLI command or a Mole clean running → Think; battery at or below the threshold and
   discharging → Sad; asleep → Sleep; else Idle.

"Claude working" = a session with `ClaudeActivity` in the last 2 minutes
that has not sent Done or Waiting since (`CLAUDE_WORKING_FOR`).

Geometry: compact shapes widen by `robot_extra` (media +2, lyric +20,
pomodoro +26; idle holds robot + clock centred as a group). Notices are
unchanged: the 40 px robot sits exactly in the badge's square. With the
robot off every width is as before (`shape_with_robot(.., false)`).

Blink only shows on open eyes (Idle, Focus, Alert, Think, Sad); the
design's `blink` eyes (two level lines with the lashes 1.5 lower) replace
the open eyes with their shine and lashes. The ears carry the mood
(`Face::ears`): up 3 for Alert, turned outward 12 degrees asleep and 24
degrees when low; the bow turns with the right ear. The ear tips stay
inside every compact island shape (`robot_and_content_share_every_compact_shape`).
The painter builds each path once, on first use.

Related: [[bata-idle-power-budget]], [[bata-morning-greeting]],
[[studio-bata-logo-sources]]

Part of [[v3-studio-overview]]. Listed in [[index]].
