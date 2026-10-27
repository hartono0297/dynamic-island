---
tags: [fonts, build, decision]
date: 2026-10-27
---

# Studio font sources and subsetting

Fonts are built by `scripts/make-fonts.ps1` and embedded as RCDATA 203-207: DM Sans 400/500/600 and DM Mono 400/500 (see [[studio-font-loading]]).

- The italic serif display face of 3.0 (RCDATA 201-202) was dropped in v3.1 for upright DM Sans 600 titles and large clocks. Ids 201-202 stay unused.
- DM Sans comes from upstream `googlefonts/dm-fonts`, not `google/fonts`: the google/fonts build (4.004) has no `tnum` feature, so tabular figures would not work.
- DM Sans 400/500/600 and DM Mono 400/500 share name ID1 = family and ID2 = Regular, so the loader tells them apart by `usWeightClass`.
- Sources are pinned by commit and SHA-256; output is byte-identical across runs. Total font size is about 189 KB (budget 370 KB).
- The 3.0 screenshots in `assets/screenshots` still show the old italic titles; the v3.1 recapture (`src/ui/demo.rs`, `scripts/capture-screenshots.ps1`) replaces them. See [[demo-data-time-of-day]] before capturing.

Related: [[studio-palette-accent-aliases]], [[studio-tabular-figures-word-gaps]]

Part of [[v3-studio-overview]].
