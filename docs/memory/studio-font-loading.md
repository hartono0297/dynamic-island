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
- The display role (titles, large clocks) is DM Sans 600 upright with letter-spacing -2.5%, applied by `fonts::apply_tracking` (`IDWriteTextLayout1::SetCharacterSpacing`, constant `fonts::DISPLAY_TRACKING_EM`). Apply it in both the draw and the measure path, or measured widths will not match what is drawn. Like `tnum` it needs a text layout: `DrawText` cannot carry it. DM Sans tops out at weight 600.
- The layout functions are pure and have no fonts (the calendar title is the exception: the renderer measures it and passes the width in, see [[studio-calendar-day-view]]), so other boxes that hold display text are sized from widths measured once with the real fonts and kept as constants beside a test (the pomodoro ring fits "100:00" in DM Sans 600 at 48 px; the idle clock box fits "00:00" in DM Sans 600 at 16 px). Re-measure when a font file, size or tracking changes.
- Digits: [[studio-tabular-figures-word-gaps]].
- The lyric notch is sized from `Renderer::lyric_text_width`, the same layout the line is drawn with: [[lyric-notch-width]].

Part of [[v3-studio-overview]].
- Tracking adds its spacing after every character, so centered display text (clock, lyric, big time) sits about `size * 0.025 / 2` px off center (0.6 px at 48 px). Accepted, not corrected.
- `settings_layout::Estimate` has no special display factor any more: display fonts use the 0.66 em/char estimate for weight 500+, which is wider than real DM Sans 600. The sidebar brand rect was widened to the gutter so the estimate fits; keep that in mind before shrinking it.
