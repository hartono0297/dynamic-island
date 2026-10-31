---
tags: [decision, cat, theme, settings, v3.8]
date: 2026-10-31
---

# Cat colours and island surfaces (v3.8)

The notch cat has four coats and the island has five fills plus "follow the
cat". Pure data in `src/theme.rs` (`CatColor`, `CatAccessory`, `Surface`) and
`src/ui/cat_style.rs` (`CatLook`); config keys in [[settings-structure]]'s
Kucing page. The accessories and the paint order are in [[cat-accessories]].

## Coats (`CatLook`)

| Coat | Head and ears | Eyes and every eye mark | Extra |
|---|---|---|---|
| Terracotta (default) | #E2683C | #17120F | as before v3.8 |
| Calico | #F4EDE4 | #17120F | an orange #E2683C patch left, a dark #2B2420 one right |
| White | #F6F1EA | #17120F | |
| Black | #1E1B22 | amber #F2B33D | a light rim #8E7CB8, 1.6 grid units round head and ears |

"Every eye mark" = the open pills, the alert dots, the blink lines, the
happy and sleep arcs (`Ink::Espresso` becomes the coat's eye colour). The
extras beside the head (z's, notes, thought dots, the heart) stay cream.
Not routed through the coat: the app logo, exe icon, tray, README and the
Settings sidebar logo (they stay terracotta, see [[logo]]), and the
notch-peek eyes pill ([[notch-peek-eyes]]). The see-through legibility halo
([[notch-size-and-fill]]) still sits behind the head of every coat.

## Surfaces

`Surface { Auto, Espresso, Graphite, Sea, Night, Plum }`, `Auto` the default.
A file without `surface`, or with an unknown name, reads as Auto; a stored
"Espresso" or "Graphite" keeps its meaning, and the old "Black" still reads
as Espresso. Fills: Espresso #17120F and Graphite #221B17 unchanged, Sea
#132321, Night #151C2E, Plum #2A2140.

`Surface::resolve(cat, robot_on)`: Auto gives Terracotta -> Espresso,
Calico -> Sea, White -> Night, Black -> Plum, and Espresso with the cat off;
any other value is itself. **Auto never reaches `island()`**:
`island_palette(cfg)` and `Theme::of` resolve first (`island` itself reads a
stray Auto as Espresso), and the Settings preview resolves the same way.
`theme::pack` / `unpack` carry the six surfaces (a test walks every
combination).

The coloured surfaces lift their own colour the way Graphite lifts Espresso:
raised +12, raised_hi +34, track and placeholder +26, column +5, dot_off +50
on every channel (Espresso and Graphite keep their pinned values). Text stays
#F4EDE4; each surface has its own `secondary` caption colour, because
#A89A8B drops to about 4.2:1 on the coloured fills.

## Contrast (WCAG, measured by `every_concrete_surface_keeps_text_and_secondary_readable`)

| Surface | Text on surface / raised | Secondary | Secondary on surface / raised |
|---|---|---|---|
| Espresso #17120F | 16.00 / 14.30 | #A89A8B | 6.78 / 6.06 |
| Graphite #221B17 | 14.61 / 12.78 | #A89A8B | 6.19 / 5.42 |
| Sea #132321 | 14.02 / 12.03 | #A3B8B2 | 7.80 / 6.69 |
| Night #151C2E | 14.61 / 12.65 | #A7B1C8 | 7.89 / 6.83 |
| Plum #2A2140 | 13.01 / 11.05 | #B9ACD0 | 7.10 / 6.03 |

The ink on every accent stays at least 4.5:1. The status colours (sage,
ochre, plum) hold 4.5:1 on every surface; the third event colour #B07A9A fell
to 4.39:1 on the Plum surface, so the Plum surface uses #C893B4 (5.96:1 on
the surface, 5.06:1 on raised) for it.

## Config (lenient, island-owned)

`cat_color`, `cat_accessory`, `cat_follow_pointer`, `notch_peek` sit at the
end of `Config`, all in `merge_owned`. A bad value falls back per field (a
typo, a number, null) and never drops the file; the two bools read on unless
a real `false`. Old files load unchanged.

## Demo

`DI_CAT_COLOR`, `DI_CAT_ACCESSORY` and `DI_SURFACE` apply to every `--demo`
scene (an unset or `auto` surface follows the cat, as in the app); the scenes
`bata-colors` (the four coats on the surface Auto gives each) and
`bata-accessories` are in `--demo-robot`.

Related: [[bata-robot-expressions]], [[settings-structure]]
