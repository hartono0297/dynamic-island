---
tags: [decision, media, perf]
date: 2026-10-27
---

# Music progress bar redraws only while visible and playing

`Frame.progress` is built from the GSMTC timeline via `lyrics::position_at`
(`Progress::at`). The notch draws a thin accent line; the Music page draws a bar,
knob and m:ss times.

The 1 s redraw timer (`TIMER_PROGRESS` in `src/app.rs`) runs only on
`View::Panel(Page::Media)` with media playing and the position before the end;
the wait is aligned to the next second boundary (`progress_wait`). Paused, idle,
track end or other pages kill it, so idle CPU does not rise. The compact view
keeps using the equalizer timer. Do not add another always-on timer for progress.

The equalizer has 5 bars (`BAR_COUNT` in `src/ui/geometry.rs`). Each frequency
times 10 s must be a whole number of cycles, since the render clock wraps at 10 s.
Settings `PREVIEW_BARS` is typed by `BAR_COUNT`.

Related: [[studio-tabular-figures-word-gaps]].

Part of [[v3-studio-overview]]. Listed in [[index]].
