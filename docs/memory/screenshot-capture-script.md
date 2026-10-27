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
- `cleaner-mole`, `-confirm`, `-log`, `-result`, `-missing` and `cleaner-mole-done` (`notch:mole-done`) show the Mole mode with fixed data; Mole is never looked for or run, see [[mole-mode]].
- Clipboard shots: `clipboard` (the design's six cards of 48), `clipboard-search`
  ("denah" typed), `clipboard-copied` (`notch:clipboard-copied`, Bata Happy) and
  `settings-clipboard`. Their data and "now" are fixed (`demo_clips`), and the
  demo never touches the real clipboard, see [[clipboard-privacy]]. The scenes
  `clipboard-empty` and `clipboard-off` exist in `--demo` but are not shot.
