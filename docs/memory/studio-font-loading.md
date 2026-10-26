---
tags: [fonts, directwrite, gotcha]
date: 2026-10-26
---

# Font loading: DirectWrite and GDI

Implemented in `src/ui/fonts.rs`; sources and subsetting in [[studio-font-sources]].

- Never unregister the in-memory font loader while text formats created from its collection are alive.
- `DrawText` cannot carry OpenType features. Text that needs tabular figures (`tnum`) must go through a text layout; the renderers share one tabular-figures setting.
- Missing or broken fonts fall back (Segoe UI Variable, Cascadia Mono / Consolas) without crashing; `choose(role, loaded, installed)` is the pure selector.
- GDI windows use `AddFontMemResourceEx` via `register_gdi()` and then verify that GDI resolves the face.
- Fraunces is only used at 16 px and above. DM Sans tops out at weight 600.
- Open check: Fraunces It 48 width inside the pomodoro ring was not verified with the real font file.
