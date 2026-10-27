---
tags: [gotcha, demo]
date: 2026-10-26
---

# Demo data depends on the time of day

`--demo` builds its calendar and reminders around `Local::now()`, so a scene
can look different (or lose a state) depending on when it is captured. Late in
the evening, events placed relative to "now" all fit one day-view window and
"+N minutes" reminders roll over to tomorrow.

Keep at least one sample pinned far from any "now" (the 07.00 coffee keeps a
second day page), and clamp relative times so they stay on today. Check
scenes at an early and a late hour before trusting a screenshot.

Part of [[v3-studio-overview]]. Listed in [[index]].
