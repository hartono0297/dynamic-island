---
tags: [handoff, v3.3]
date: 2026-10-30
---

# v3.3: where it stands

Plan: `docs/superpowers/plans/2026-10-29-v3.3.md`. Made the way the owner
asked: planned by Opus, built by four Sonnet builders in parallel worktrees
(one per task), reviewed and fixed by Opus, then merged to `main`.

- **Taskbar icon** ([[taskbar-icon-from-exe]]): the exe already had the cat;
  Explorer's icon cache kept the old one. The app tells the shell once per
  version, the installer also refreshes shortcuts and runs `ie4uinit -show`.
- **Stay-behind rise** ([[stay-behind]]): fade and slide in 180 ms (fade only
  in 120 ms with Saver or Energy Saver), only when the island was covered.
- **Shelf drop** ([[shelf]]): a drop opens the shelf and latches it like a
  click; in Hover mode normal rules return on the next visit.
- **Search** ([[search-files]]): apps, files and folders; six user folders,
  depth 5, 10 000 entries per root and 30 000 in all; files from two
  characters; icons, sizes, folder counts and app sizes from a background
  worker. The review found that OneDrive folders were skipped (cloud reparse
  tags are not links); fixed.
- **Google sign-in guide** for the owner: `docs/google-login-setup.md`.
- The whole repo was `cargo fmt`-ed once (it had drifted on the other PC).

Released as v3.3.0 on 2026-10-30 (owner's go-ahead); `Cargo.toml`, `app.manifest` and `AppxManifest.xml` say 3.3.0.

Owner checks no agent could do: the icon on taskbar, Alt+Tab, Start Menu and
a pinned button after reinstalling; the rise with stay-behind on (smooth, no
blink when uncovered); a drop keeping the shelf in all three open modes; file
search on real folders (OneDrive online-only files must not download, a large
Documents must stay responsive), app install sizes, icon edges in light and
dark, both languages.

Minor findings left: the "Membaca berkas…" hint is almost never visible;
unused `SearchKindFile`/`SearchKindFolder` keys; sizes just under 10 MB read
"10.0 MB"; a details read can run once after the search closes; a panicking
walk would leave `walking` set; ranking sorts all hits per key; shell icons
may be premultiplied twice (check icon edges); a drop left without a
pointer move takes one extra visit to hand over to hover.

Related: [[v3-2-handoff]], [[readme-structure]]
