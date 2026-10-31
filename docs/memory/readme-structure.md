---
tags: [convention, readme, docs, v3.2]
date: 2026-10-29
---

# README: front page with a gallery, long guide

`README.md` (reworked 2026-10-29 on the owner's ask: "too little", and
uninstall must be there too): logo, pitch, badges (release, downloads, stars,
MIT, Windows 11, size), quick links, the animated tour, *Why Vale*, Install,
**Uninstall** (the `irm` line, `DI_PURGE=1` to remove data, the offline copy
in `%LOCALAPPDATA%\Programs\Vale`), the feature table, a 2 × 4 screenshot
gallery (`<table>`), the notices animation, Privacy, Build, **Credits** and
License. The long reference (every feature, settings table, Google Calendar
setup, Mole, data and privacy, troubleshooting, limits) lives in
`docs/guide.md`; new detail goes there.

- **Credits:** `THIRD-PARTY-NOTICES.md` lists every third-party component:
  Mole for Windows (data, MIT at `500dfc4c`; runs your own GPL-3.0 install),
  DM Sans / DM Mono (OFL 1.1), every Rust crate from `Cargo.lock` with version
  and licence, the services (Google Calendar, LRCLIB, Google Search, GitHub
  Releases), Claude Code, and the trademark note (inspired by Apple's Dynamic
  Island). Regenerate its crate tables when `Cargo.lock` changes:
  `cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc > meta.json`
  then `python scripts/third-party.py meta.json`. `NOTICE` keeps Mole's full
  MIT text. The release zip carries `LICENSE`, `NOTICE`,
  `THIRD-PARTY-NOTICES.md` and the fonts' OFL as `OFL-fonts.txt`.
- **Animations:** `assets/screenshots/tour.png` (pages) and `notices.png` are
  **APNG**, which GitHub shows as a looping animation in an `<img>`. They are
  built from the demo screenshots by `scripts/make-apng.py` (standard-library
  Python, cross-fades between shots): run `capture-screenshots.ps1` first when
  the pages changed, then `python scripts/make-apng.py tour assets/screenshots/tour.png`
  and `... notices ...`. The shot lists are in `TOURS` in the script. `notices`
  holds notices only: a whole-panel shot (`notch`) shows cropped in its 160 px
  canvas. No video:
  a repo README cannot embed an mp4 without uploading it elsewhere.
- **Stars badge** is shields.io (`github/stars`), live; it reads 0 until the
  repo is starred.
- **Licence:** MIT, © 2026 hartono0297 (`LICENSE`, `Cargo.toml` `license`),
  chosen by the owner on 2026-10-29. `NOTICE` and `assets/fonts/OFL.txt` keep the
  third-party licences. The release zip carries `LICENSE`.

Related: [[screenshot-capture-script]], [[v3-2-handoff]].

## v3.8 additions

The "Yours to shape" and cat bullets, "What it does" and the closing
Settings paragraph name the v3.8 features: time-of-day greeting
([[greeting-periods]]), four coats and seven accessories
([[cat-colours-and-surfaces]], [[cat-accessories]]), the island background
(Match cat plus five colours), the follow-pointer and shrink-to-eyes
switches, and the eight Settings sections ([[settings-structure]]). Gotcha:
`docs/guide.md` (greeting line 73, Settings table around line 124) still
describes the old morning greeting and six sections; README leads, the guide
is behind.
