---
tags: [decision, shortcuts, settings, config, v3.2]
date: 2026-10-28
---

# Shortcut tiles you choose (v3.2 Run 7)

`src/tiles.rs` (`ShortcutTiles`, like `pages::PanelPages`): the Pintasan page
shows the tiles in `config.shortcut_tiles`, in that order, up to twelve (four
per row, up to three rows, `panel_layout::shortcuts_layout` wraps). Empty =
the eight built-ins, so old configs read as before. Settings → Tampilan →
**Pintasan** has a row per tile (toggle, order stepper, "Hapus" on a custom
one); the last tile shown cannot be switched off.

- **Built-in** entry: `{"tile":"Lock","on":false}`; each built-in is listed
  once, one missing from the file rejoins right after the one before it in
  `Shortcut::ALL`.
- **Custom** entry: `{"tile":"Custom","label","kind","target"}`; `kind` is
  `App`, `Folder` or `Url`. `CustomTile::new` is the one gate: label 1–14
  chars, target ≤ 500, no control characters; a `Url` must be http(s) with a
  host or `mailto:` (no `file:`, `javascript:`, `ms-*:`). An entry that fails
  is dropped when read, never kept to fail at click time. Max 8 custom. An
  absent `on` reads as shown.
- Click path: `Hit::Shortcut(s)` (built-in, as before) and
  `Hit::CustomTile(i)` (`i` = place among the tiles shown) →
  `Effect::RunCustom` → `Action::Custom` → `ShellExecute "open"` on the
  target, after the panel collapses like any tile. `Action` is no longer
  `Copy` (it owns a string).
- The settings scroll now counts text under the last card (the note under
  the Pintasan card), not just card bottoms.

**Not done (decision for the owner):** an *add* form in Settings (needs two
more native edit controls and an IME-safe focus path; untested without a live
run) and the "Perintah" kind (a shell command per tile: a security surface,
the plan wanted a first-run confirmation). Own tiles are added in
config.json for now (README shows how).

Related: [[open-mode]].
