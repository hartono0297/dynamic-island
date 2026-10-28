---
tags: [convention, logo, assets]
date: 2026-10-29
---

# The Vale logo lives in three places that must match

The cat head (v3.2.1, see [[logo]]; it replaced the v3.1 "eared tile"): the
notch cat's 64 grid scaled by 4.6 into the 256 space. Head #E2683C, a dome on
a flat base (x 13..243, y 97..235, bottom corners 18.5), two ears (triangles
(22,157) (36,32.5) (105,106) and mirrored, a 23-unit round stroke) and two
pill eyes #17120F (26 x 46 at x 78 and 152, y 147.5). Fixed brand colours,
independent of the user accent. The same numbers are in `assets/logo.svg`,
`scripts/make-icon.ps1` (which regenerates `assets/icon.ico` and
`assets/logo-256.png`) and `fn logo` / `head_geometry` / `ears_geometry` in
`src/ui/settings_controls.rs`. Change all three together.

The 16/20/24/32 px sizes snap the eyes to whole pixels (2 x 4 at 16 px) so
they stay crisp; head and ears just scale. `site/img/logo.svg` is a copy of
`assets/logo.svg`.

The cat in the notch uses the same brand colours (no bow since 2026-10-28), see
[[bata-robot-expressions]].

Related: [[studio-palette-accent-aliases]]

Part of [[v3-studio-overview]].
