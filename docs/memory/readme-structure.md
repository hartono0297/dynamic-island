---
tags: [convention, readme, docs, v3.2]
date: 2026-10-29
---

# README: short front page, long guide

`README.md` is the minimalist front page: logo, one-line pitch, badges
(release, stars, MIT, Windows 11, size), an animated tour, the install line, a
one-screen feature table, a pointer to the guide, the privacy line and the
licence. The long reference (every feature, settings table, Google Calendar
setup, Mole, data and privacy, build, troubleshooting, limits, fonts and
credits) lives in `docs/guide.md`. Keep the README short; put new detail in
the guide, and update the README table only for a new page or headline feature.

- **Animations:** `assets/screenshots/tour.png` (pages) and `notices.png` are
  **APNG**, which GitHub shows as a looping animation in an `<img>`. They are
  built from the demo screenshots by `scripts/make-apng.py` (standard-library
  Python, cross-fades between shots): run `capture-screenshots.ps1` first when
  the pages changed, then `python scripts/make-apng.py tour assets/screenshots/tour.png`
  and `... notices ...`. The shot lists are in `TOURS` in the script. No video:
  a repo README cannot embed an mp4 without uploading it elsewhere.
- **Stars badge** is shields.io (`github/stars`), live; it reads 0 until the
  repo is starred.
- **Licence:** MIT, © 2026 hartono0297 (`LICENSE`, `Cargo.toml` `license`),
  chosen by the owner on 2026-10-29. `NOTICE` and `assets/fonts/OFL.txt` keep the
  third-party licences. The release zip carries `LICENSE`.

Related: [[screenshot-capture-script]], [[v3-2-handoff]].
