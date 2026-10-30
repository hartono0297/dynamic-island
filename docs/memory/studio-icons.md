---
tags: [icons, direct2d, decision]
date: 2026-10-27
---

# Studio icons: Direct2D vector set

Implemented in `src/ui/icons.rs`. The shapes come from `icon()` in `docs/design/studio-ui.html`. This replaced the Segoe Fluent Icons / Segoe MDL2 Assets glyphs. Fonts are covered in [[studio-font-loading]].

- **The stroke width stays in DIPs.** Each icon is drawn with `CreateTransformedGeometry` (scale size/24 plus a translation) and stroked at 1.5. A transformed geometry is stroked after its transform, so a 12 px chevron and a 28 px play button both get a 1.5 DIP line. The render target's DPI and world transform still apply.
- **The duotone fill is off on accent backgrounds.** The reminder bell sits on the accent badge, so it draws in `on_accent` with no fill. The settings preview's music note on the accent art does the same. Everywhere else the fill is the accent at 0.38 × the ink alpha, so it fades along with the icon.
- Geometries are built once per `ID2D1Factory`, in three layers (duo fill, stroke, dots). They are device independent, so a lost target needs no rebuild.
- Gotcha: a `D2D1_FIGURE_BEGIN_HOLLOW` figure has empty bounds (`GetBounds` returns inf). All figures therefore begin FILLED, including the stroke layer, which is never filled.
- SVG arc sweep flag 1 maps to `D2D1_SWEEP_DIRECTION_CLOCKWISE` (y down). The bell's dome rising to y = 5 is the unit test for this.
- **Adding an icon:** copy its path from the design's `icon()` (24-unit grid) rather than drawing a new one, keep strokes at 1.5, and decide whether it can sit on the accent (then no duotone fill). Bata is drawn separately, see [[bata-robot-expressions]].

**v3.5:** `Usb`, `Hdd`, `Sd` and `Eject` ([[devices]]) come from `ic()` in `docs/design/rencana-v3.5.html` (page 03). Line only, no duotone. The design draws the zero-length "dots" (`M11 5h0`, `h.01`) as round-capped paths; here they are `Part::Dot`s, since a zero-length segment is not reliably drawn.

Part of [[v3-studio-overview]].
