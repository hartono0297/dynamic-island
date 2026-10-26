---
tags: [gotcha, fonts, render]
date: 2026-10-26
---

# DM Sans `tnum` widens the space character

DM Sans's tabular-figures feature also widens the space, so text with `tnum`
on the whole line shows wide gaps between words. The settings window avoids it
by setting `tnum` only on the digit runs (`Font::tabular` in
`src/ui/settings_controls.rs`).

Open: the island renderer (`src/ui/render.rs`) still applies one tabular
typography to every line, so it probably has the same gaps. Check it and apply
the digits-only approach there if it shows.

See [[studio-font-loading]].

Listed in [[index]].
