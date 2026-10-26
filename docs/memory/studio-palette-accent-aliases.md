---
tags: [decision, theme, config]
date: 2026-10-26
---

# Studio palette: old accent names are aliased

The v3 Studio palette replaced the accent and surface variants. To keep old
`config.json` files from silently reverting to defaults, `named_enum!` takes an
`aliases { "Old" => New }` list that `from_name` checks after the real names
(trimmed, case-insensitive). Saving always writes the new name.

Mapping: Blue and Red to Terracotta, Green to Sage, Orange to Ochre, Pink to
Clay, Purple to Plum, Black to Espresso. Unknown names still fall back to the
default.

Related: [[studio-light-secondary-text-contrast]], [[studio-bata-logo-sources]]
