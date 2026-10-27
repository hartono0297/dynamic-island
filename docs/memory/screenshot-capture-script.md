---
tags: [screenshots, convention]
date: 2026-10-27
---

# Screenshot capture script

`scripts/capture-screenshots.ps1` regenerates `assets/screenshots/*.png` from the debug build (`DI_LANG=en`) via the demo modes in `src/ui/demo.rs`. It copies each window over black then white and derives alpha from the difference, so rounded shapes stay transparent. Options: `-Only`, `-Out`, `-Window dark|light`.

- Calls SetProcessDPIAware first, see [[studio-editor-frame]].
- Demo data depends on the clock, see [[demo-data-time-of-day]].
- Demo titles (events, notes, reminders) stay Indonesian even with `DI_LANG=en`.
