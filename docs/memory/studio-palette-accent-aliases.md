---
tags: [decision, theme, config]
date: 2026-10-27
---

# Studio palette: old accent and surface names are aliased

The v3 Studio palette replaced the accent and surface variants. To keep old
`config.json` files from silently reverting to defaults, `named_enum!` takes an
`aliases { "Old" => New }` list that `from_name` checks after the real names
(trimmed, case-insensitive). Saving always writes the new name, so an old file
is upgraded on its first save.

Mapping:

- Accent: Blue and Red to Terracotta, Green to Sage, Orange to Ochre, Pink to
  Clay, Purple to Plum. Olive is new and has no old name.
- Surface: Black to Espresso (Graphite is new).

Unknown names still fall back to the default. When a variant is renamed or
dropped again, add an alias instead of deleting the old name; the same macro
serves other settings enums (the robot's motion policy), so they can alias too.

Related: [[studio-light-secondary-text-contrast]], [[studio-bata-logo-sources]]

Part of [[v3-studio-overview]].
