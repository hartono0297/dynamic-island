---
tags: [handoff, v3.5]
date: 2026-10-30
---

# v3.5: where it stands

Plan: `docs/design/rencana-v3.5.html`. Every change went through the `build`
workflow with `defer: true`, then one verify pass (blind review + check) at
the end. `Cargo.toml`, `app.manifest` and `AppxManifest.xml` say 3.5.0; no
tag pushed (release stays with the owner).

- **Editor close bar** ([[shot-editor-paint-order]]): Discard (red) and
  Cancel are painted after the footer, Enter discards, Esc keeps, X and
  Alt+F4 ask too.
- **Pages** ([[github-pages]]): `.github/workflows/pages.yml` publishes
  `site/`.
- **Devices page** ([[devices]]): own page, notices on by default, hotkey
  Ctrl+Alt+D off by default. Verify found two issues after three fix rounds
  (a leftover `pagefile.sys` hid an old system disk; an unplugged drive got a
  "safe to remove" notice with a raw instance ID); fixed in a separate run.
- **Notch size and see-through fill** ([[notch-size-and-fill]]): 190 x 34 by
  default, 160-300 x 28-44, opacity 30 % in Transparan; the open panel keeps
  its own background.
- **Shelf motion** ([[shelf-motion]]) and **clipboard copy notice**
  ([[clipboard-notice]], on by default).

Cancelled by the owner (prototypes kept outside the repo, in the session
notes folder):

- GPU/VRAM/CPU monitor, ADLX/NVML and the game strip: the strip cannot show
  over exclusive fullscreen without injecting into the game. The ADLX
  prototype itself worked (RX 6700 XT: watts, temp, hotspot; FPS returned
  NOT_SUPPORTED with no game; first init blocks ~0.7 s, so a worker thread).
- Transparent taskbar via TranslucentTB: the settings file
  (`%LOCALAPPDATA%\Packages\28017CharlesMilette.TranslucentTB_v826wp6bftszj\RoamingState\settings.json`)
  is stable and hot-reloaded, but the owner set the feature aside.

Owner checks no agent could do: the editor bar by hand (`--demo-shot-editor`);
Devices with a real USB drive and SD card (eject, veto message, notices, USB
speed); notch sliders, hover at 0 % and legibility over a bright photo
wallpaper; a real Explorer drop and X/Kosongkan mid-animation on the shelf;
real copies from other apps and password managers for the clipboard notice.
Owner steps: Settings -> Pages -> Source: GitHub Actions; capture
`assets/screenshots/devices.png`.

Open findings: none.

Related: [[v3-3-handoff]]
