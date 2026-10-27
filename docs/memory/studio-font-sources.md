---
tags: [fonts, build, decision]
date: 2026-10-27
---

# Studio font sources and subsetting

Fonts are built by `scripts/make-fonts.ps1` and embedded as RCDATA 201-207 (see [[studio-font-loading]]).

- DM Sans comes from upstream `googlefonts/dm-fonts`, not `google/fonts`: the google/fonts build (4.004) has no `tnum` feature, so tabular figures would not work.
- Fraunces at wght 500 / opsz 20 swaps in non-wonky `.alt` glyphs through `rvrn`. Not every renderer applies `rvrn` to a static font, so the script writes those swaps into the cmap.
- DM Sans 400/500/600 and DM Mono 400/500 share name ID1 = family and ID2 = Regular, so the loader tells them apart by `usWeightClass`.
- Sources are pinned by commit and SHA-256; output is byte-identical across runs. Total font size is about 296 KB (budget 370 KB).
- Still open at 3.0.0: the screenshots in `assets/screenshots` date from v2.2 and must be regenerated (`src/ui/demo.rs`) to show the new fonts, palette, icons and robot. See [[demo-data-time-of-day]] before capturing.

Related: [[studio-palette-accent-aliases]], [[studio-tabular-figures-word-gaps]]

Part of [[v3-studio-overview]].
