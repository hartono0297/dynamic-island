---
tags: [decision, rename, install]
date: 2026-10-28
---

# The app is Vale now; what still says Dynamic Island, and why

Run 12 of the v3.1 plan. The display name, exe (`vale.exe`), install folder
(`%LOCALAPPDATA%\Programs\Vale`), data folder (`%APPDATA%\Vale`), pipe
(`\\.\pipe\vale`), Run value, window classes, tray tooltip and log prefix all
say Vale.

- **Data moves on first start** (`src/data_dir.rs`): rename `DynamicIsland` to
  `Vale`; if that fails (a file held open), copy into `Vale.migrating` and
  rename that; a note `MOVED-TO-Vale.txt` is left in the old folder. If the
  copy fails too, the half copy is removed and the app keeps using the old
  folder (`dir()` picks the old one while the new one is missing).
- **Old names kept on purpose**: the `Local\DynamicIsland` mutex is claimed next
  to `Local\Vale`, so an old and a new build never run together; Claude hooks
  that call `dynamic-island.exe --hook` are recognised, repointed and removed
  like Vale's; the old `DynamicIsland` Run value is always deleted at start.
- **install.ps1 / uninstall.ps1** stop both process names and remove the old
  program folder, Start Menu shortcut and Run value. Data stays unless
  `DI_PURGE=1` (the variable kept its name).
- **Sparse package**: Identity and application id are `Vale`, but the
  publisher stays `CN=DynamicIslandDev`, because it must match the dev
  certificate the owner already trusts (re-trusting needs admin).
  `package.ps1` removes an old `DynamicIsland` registration.
- **Repo URLs** (`hartono0297/dynamic-island`) stay until the owner renames
  the GitHub repo; GitHub redirects the old URLs after a rename.
- `scripts/make-fonts.ps1` keeps the `dynamic-island-subset` tag in the font
  names, since changing it changes the font files for no visible gain.

- Verify pass: an empty `%APPDATA%\Vale` (any app may make one) is removed and
  the data moves in; a non-empty one is still taken as Vale's. The old
  `Run\DynamicIsland` value is deleted only when it starts
  `dynamic-island.exe`.

Related: [[v3-1-handoff]], [[v3-studio-overview]]
