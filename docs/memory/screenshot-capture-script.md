---
tags: [screenshots, convention]
date: 2026-10-27
---

# Screenshot capture script

`scripts/capture-screenshots.ps1` regenerates `assets/screenshots/*.png` from the debug build (`DI_LANG=en`) via the demo modes in `src/ui/demo.rs`. It copies each window over black then white and derives alpha from the difference, so rounded shapes stay transparent. Options: `-Only`, `-Out`, `-Window dark|light`.

- Calls SetProcessDPIAware first, see [[studio-editor-frame]].
- Demo data depends on the clock, see [[demo-data-time-of-day]].
- `lyric-long` freezes the long lyric line mid-scroll (`DI_LYRIC_SCROLL`), see [[lyric-notch-width]].
- `system` and `system-warning` (`notch:bata-sistem`) use fixed figures, never this machine's; see [[system-page]].
- Demo titles (events, notes, reminders) stay Indonesian even with `DI_LANG=en`.
- `cleaner`, `cleaner-confirm` and `cleaner-done` show the cleaner with the design's fixed sizes; the demo never scans or deletes, see [[cleaner]].
