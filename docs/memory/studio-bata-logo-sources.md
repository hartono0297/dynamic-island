---
tags: [convention, logo, assets]
date: 2026-10-26
---

# Bata logo lives in three places that must match

Tile #E2683C, pill #17120F (rect 58,104,140,48 rx 24 in the 256 space), dot
#F4EDE4 (circle 174,128 r9.5). Fixed brand colours, independent of the user
accent. The same numbers are in `assets/logo.svg`, `scripts/make-icon.ps1`
(which regenerates `assets/icon.ico` and `assets/logo-256.png`) and
`fn logo` in `src/ui/settings_controls.rs`. Change all three together.

The 16/20/24/32 px sizes use a hand-snapped table in `make-icon.ps1` so the pill
and 2 px dot stay crisp; larger sizes are computed.

Related: [[studio-palette-accent-aliases]]
