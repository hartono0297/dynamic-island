---
tags: [convention, logo, assets]
date: 2026-10-28
---

# The Vale logo lives in three places that must match

The "eared tile" (v3.1, Run 13): tile #E2683C (rect 12,52,232,192 rx 72 in
the 256 space) with two ears (triangles (30,104) (52,22) (116,64) and
mirrored, a 16-unit round stroke), pill #17120F (rect 58,122,140,48 rx 24),
dot #F4EDE4 (circle 174,146 r9.5). Fixed brand colours, independent of the
user accent. The same numbers are in `assets/logo.svg`, `scripts/make-icon.ps1`
(which regenerates `assets/icon.ico` and `assets/logo-256.png`) and
`fn logo` in `src/ui/settings_controls.rs`. Change all three together.

The 16/20/24/32 px sizes use a hand-snapped table in `make-icon.ps1` (tile
inset and top, pill, 2 px dot) so they stay crisp; the ears follow the snapped
tile. Larger sizes are computed. `site/img/logo.svg` is a copy of
`assets/logo.svg`. The 16 px icon reads as a cat on dark and light taskbars.

The cat in the notch uses the same brand colours plus the plum bow, see
[[bata-robot-expressions]].

Related: [[studio-palette-accent-aliases]]

Part of [[v3-studio-overview]].
