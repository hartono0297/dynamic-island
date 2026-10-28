---
tags: [handoff, v3.2]
date: 2026-10-28
---

# v3.2 handoff

Version `3.2.0` (Cargo.toml, Cargo.lock), `cargo test --locked` 966 passed. Not
pushed, not tagged: pushing `main` and the `v3.2.0` tag are the owner's calls.

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
default (Off now), the open-mode default (Hover now) and the cat variant.

**Needs live checks** (the code was written and unit-tested, never run against
the real thing): Windows clipboard formats of Snipping Tool and the preview;
real YouTube Music / Spotify positions for lyrics; click behaviours (media
press/release guard, outside-click poll); IME in the search and clipboard
fields; hotkey conflicts; fullscreen focus; Core Audio sessions and drag feel
in the Mixer; OLE drop onto the layered window and drag-out of the shelf;
a touchpad's sideways scroll.
