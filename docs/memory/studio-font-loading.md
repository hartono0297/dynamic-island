---
tags: [fonts, directwrite, gotcha]
date: 2026-10-27
---

# Font loading: DirectWrite and GDI

Implemented in `src/ui/fonts.rs`; sources and subsetting in [[studio-font-sources]].

- Never unregister the in-memory font loader while text formats created from its collection are alive.
- `DrawText` cannot carry OpenType features. Text that needs tabular figures (`tnum`) must go through a text layout; the renderers share one tabular-figures setting.
- Missing or broken fonts fall back (Segoe UI Variable, Cascadia Mono / Consolas) without crashing; `choose(role, loaded, installed)` is the pure selector.
- GDI windows use `AddFontMemResourceEx` via `register_gdi()` and then verify that GDI resolves the face.
- Fraunces is only used at 16 px and above. DM Sans tops out at weight 600.
- The layout functions are pure and have no fonts (the calendar title is the exception: the renderer measures it and passes the width in, see [[studio-calendar-day-view]]), so other boxes that hold display text are sized from widths measured once with the real fonts and kept as constants beside a test (the pomodoro ring fits "100:00" in Fraunces It 48; the idle clock box fits "00:00" in Fraunces It 16). Re-measure when a font file or size changes.
- Digits: [[studio-tabular-figures-word-gaps]].

Part of [[v3-studio-overview]].
