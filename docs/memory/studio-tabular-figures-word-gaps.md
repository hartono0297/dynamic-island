---
tags: [gotcha, fonts, render]
date: 2026-10-27
---

# DM Sans `tnum` widens the space character

DM Sans's tabular-figures feature also widens the space (U+0020), so `tnum` on
a whole line shows wide gaps between words ("Minum  vitamin").

Rule: apply `tnum` to ASCII digit runs only. `fonts::text_layout` does this with
one `DWRITE_TEXT_RANGE` per run from `fonts::digit_runs` (lazy, UTF-16 offsets),
so clock and timer digits stay tabular while spaces stay normal. The settings
window has its own digit-run path (`Font::tabular` in
`src/ui/settings_controls.rs`). Never pass a whole-line tabular typography to
SetTypography.

A test builds a real DirectWrite layout and checks the typography lands on digits
only and that a no-digit string measures the same as with no features.

See [[studio-font-loading]].

Listed in [[index]].
