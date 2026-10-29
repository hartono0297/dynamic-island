---
tags: [handoff, v3.2]
date: 2026-10-28
---

# v3.2 handoff

**State (2026-10-29): version 3.2.1 is released** (tags `v3.2.0` and `v3.2.1`
pushed by the owner, release assets built by `release.yml`; `install.ps1` installs
the latest). `main` is pushed; `cargo test --locked` 1049 passed. To continue on
another PC: `git pull`, read this note and [[index]], `rustup` + MSVC, then
`cargo test --locked` (the Mole PowerShell test is a known flake under load;
rerun). Tag pushes are the owner's call (the agent's permission check refuses
them): the owner runs `git tag vX.Y.Z; git push origin vX.Y.Z` in PowerShell
(no `&&` in Windows PowerShell 5.1).

Done, one commit per run (plan: `docs/design/rencana-v3.2.pdf`):
[[open-mode]] (Mode buka, notices with buttons), clipboard scroll, screenshot
preview ([[screenshot-preview]]), [[lyrics-sync]], Google messages
([[google-built-in-client]]), the flat-based cat ([[bata-robot-expressions]]),
[[shortcut-tiles]], [[gestures]], [[mixer]], [[search]], [[shelf]].

**Not done:** "Di belakang jendela" (Run 12, the plan's optional experiment,
off by default): it fights over z-order with fullscreen apps and games and
could leave the island unreachable, so it waits for a live test and the
owner's say. File search in the quick search; a form to add custom shortcut
tiles in Settings (config.json for now) and a "Perintah" tile kind.

**Owner steps:** register the Google Cloud project and set the two repository
secrets (login without Cloud Console for users depends on it); install
superpowers (`claude plugin install superpowers@claude-plugins-official
--scope user`; the classifier denied the agent); decide the search hotkey
default (done in 3.2.1: Ctrl+Alt+Space), the open-mode default (Hover now) and the cat variant.

**Needs live checks** (the code was written and unit-tested, never run against
the real thing): Windows clipboard formats of Snipping Tool and the preview;
real YouTube Music / Spotify positions for lyrics; click behaviours (media
press/release guard, outside-click poll); IME in the search and clipboard
fields; hotkey conflicts; fullscreen focus; Core Audio sessions and drag feel
in the Mixer; OLE drop onto the layered window and drag-out of the shelf;
a touchpad's sideways scroll.

## 3.2.1 (2026-10-29): fixes after the owner's first week with 3.2.0

Plan: `docs/superpowers/plans/2026-10-29-v3.2.1.md`. Done: clipboard card
buttons follow the pointer ([[clipboard-history]]); wheel volume on the notch
([[scroll-volume]]); lyrics never early ([[lyrics-sync]]); the shelf opens on
drag-over and a tidier page ([[shelf]]); the System page eases and samples
only while shown ([[system-page]]); stay-behind-windows, experimental and off
([[stay-behind]]); the search on by default with a tray entry and a Pintasan
tile ([[search]]); the cat head logo everywhere ([[logo]]). A blind review of
the whole diff found eight real issues; all were fixed (hotkey setting that
did not stick, a drag margin that blocked clicks, a raise strip as wide as the
window, tiles pushed out of the 12, tray search keyboard, System request
counter, stale volume points, two redraws).

Still unverified live: a real Explorer drag onto the idle island (the halo
appears from a 50 ms poll while the left button is down near it), the 20 Hz
wake-ups of that poll, `HWND_BOTTOM` above wallpaper and icons, master volume
on a PC with sound, lyric timing on Spotify/YouTube Music, a refreshed tray
icon (Windows caches icons; restart Vale, maybe Explorer).

### 3.2.1 review rounds (2026-10-29)

Two rounds of read-only reviews (window and pointer, clipboard/screenshot/shelf,
audio/lyrics, search/tiles/settings, security, regressions in older features,
idle CPU, System/logo/docs) found and fixed about 60 real issues; each fixer
worked in its own worktree and was merged into `main` (conflicts resolved by
hand in `screenshot/win.rs`, `google/http.rs`, `app.rs`). Highlights: idle
main-thread wake-ups back near 3.1.0 (the halo poll starts only from a trigger),
notices no longer hide an open panel, Simpan/Hapus wait for Snipping Tool's
file and never overwrite or permanently delete, custom tiles need absolute
local paths, lyric parsing and downloads are bounded, Mixer rows stay in
place, the search cleans up after itself. The second round found only small
things (calculator `0.001`, CPU ease step, Hapus own file, a forged OAuth code).
Tests: 1049. Sub-agents ran on the default model: the `opus` alias failed in
this environment ("model not found").

