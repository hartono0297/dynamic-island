---
tags: [decision, cleaner, mole, licence, notice]
date: 2026-10-28
---

# The cleaner's "Otomatis · Mole" mode

Run 6 of the v3.1 plan (keputusan-v3.1 decision 4b, option D;
`docs/design/deepclean-opsi-d.html` d1, d2). Engine in `src/mole.rs`, the
phase in the reducer (`state::MolePhase`), the panel in
`panel_layout::mole_layout` and `render_panel::paint_mole`. Same canvas as
the own mode ([[cleaner]]), so switching modes does not morph; the mode last
used is `clean_mode` in config (serde default `Own`).

- **Detection** (on the Mole worker, PATH may be slow): `%LOCALAPPDATA%\Mole\mole.ps1`
  first, then `mo` on PATH (`mo.ps1`, then `.cmd`/`.bat`/`.exe` in PATHEXT
  order). `VERSION` beside it, trimmed. Only 1.30.x is `tested()`; another
  or missing version shows an ochre warning line and may still run. Not found:
  the "Cara memasang" card, a link to Mole's Windows README. Never installs.
  `detect(&MoleEnv)` is injectable; tests never call `detect_here()`.
- **Gate, in types and in the reducer.** `run_clean` needs a `Preview` (only
  `run_preview` makes one, from a dry-run that exited 0, for its options) and
  a `Confirmed` (made only where `Effect::MoleClean` is handled). Other
  options than the preview's: `Stale`, nothing runs. The reducer arms
  "Jalankan Mole" only in `Previewed { opts: <ticked now>, ok: true }`; the
  second click within `CLEAN_CONFIRM` (shared with the own mode, so the same
  `next_deadline`) sends `MoleClean`. Any other click, an option tick (which
  also starts a new preview), the timeout or the panel closing disarms it.
  The app also drops a preview whose options differ. Tests: a fake
  `MoleRunner` records every argv; reducer tests count clean effects.
- **Processes:** hidden `powershell -NoProfile -ExecutionPolicy Bypass -File
  mole.ps1 clean [--dry-run] [--game-media]` (`CREATE_NO_WINDOW`, stdin null,
  stdout+stderr in one pipe, in a kill-on-close Job like the CLI). A preview
  is killed when the cleaner closes or leaves Mole mode; a clean is never
  killed and goes on with the panel closed, its notice still comes. A run
  counter drops lines and results of a superseded run.
- **Parse rules:** each line is ANSI-stripped (CSI, OSC, 8-bit CSI, control
  chars, the last `\r` segment). A section starts at "▸ Name" (`?`/U+FFFD
  stand in for a lost ▸); its name maps to Browser / Apps / Dev / TempLogs by
  keywords. Items are only "Name (N items, X MB dry)" (`,` or `.` decimals,
  B..TB, binary units). Summary words (Total, reclaimable, …) never count;
  Mole's summary is known to be wrong. Exact duplicates count once. System and
  game-media sections only with their option ticked. A group with no readable
  line is `Unreadable` → "lihat log", never a number; a failed preview shows
  every group that way and the button stays off.
- **The log** drops empty lines, rules/box drawing and Mole's internal
  Size / Removed / Failed table rows ("Failed to remove …" stays). At most
  `LOG_CAP` lines, drawn from the top in DM Mono 9.5, the tail once it is full.
- **Result = measured free space,** `GetDiskFreeSpaceExW` on the worker,
  before and after, for the system drive and any drive a preview line named
  (`clean::measure_freed`). Notice `MoleDone` (Bata Happy, sage badge) or
  `MoleFailed` (sticky, Bata Sad, ochre badge); both have a "Log" chip and
  open the cleaner on the log (`mole_pending`). While it runs Bata thinks
  (`Brain::mole_running`), see [[bata-robot-expressions]].
- **Admin via runas:** "+ Sistem (admin)" is shown, never ticked by itself.
  The user part runs hidden first; then `clean --system` runs in a separate
  PowerShell through `ShellExecuteExW` verb `runas` (hidden, output to a temp
  log read back, UTF-16 or UTF-8). A refused UAC prompt is "skipped", not a
  failure. `app.manifest` stays asInvoker. Gotcha (Mole docs): without admin,
  `--system --dry-run` previews only the user part, so the hidden preview may
  show no system lines; and an admin who is another user has their own
  whitelist.
- **Whitelist defaults:** before the first preview, append the Cargo caches
  (`.cargo\registry`, `.cargo\git`) and `Autodesk` under Local and Roaming
  AppData to `%USERPROFILE%\.config\mole\whitelist.txt` (Mole's Windows
  reference). Append-only, deduplicated without case or trailing slash
  (comments do not count), a new file gets a UTF-8 BOM (PowerShell 5.1), a
  missing final newline is added first, written via a temp file. Done once
  (`mole_defaults_added`), so an entry the user removes is never added back.
  Settings → Umum toggle `mole_protect_defaults` (serde default true).
  "Lindungi folder…" opens `clean --whitelist` in a visible console.
- **Licence boundary:** the island only RUNS the Mole the user installed, as
  its own process; no Mole code is copied, vendored or linked. Mole's
  `windows` branch is GPL-3.0 since 2026-10-11 (the own mode's location data
  is from the last MIT commit, see [[cleaner]]). README says so in its Mole
  section and credits.
- **Notice width fix:** the Notice shape stays 360 × 76 for every notice.
  Texts pick a wording by a measured `fits` (`Renderer::notice_text_fits`,
  DirectWrite with the embedded DM Sans): `i18n::fit_parts` tries every part
  long, then every part short, then drops the least important part.
  "Disk C: sekarang sisa 49,5 GB · 3 item dilewati (sedang dipakai)" (305 px
  in a 270 px body) becomes "C: sisa 49,5 GB · 3 item dilewati". Mole's
  notices use a narrower "Log" chip (46 px); the title falls back to
  "Mole · X dibebaskan", then "X dibebaskan". Tests measure ID and EN.

Demo scenes `cleaner-mole*`, `mole-done`, `mole-failed` use fixed data and
never look for Mole; shots in [[screenshot-capture-script]]. The mode lives
on the [[system-page]]'s cleaner.

Listed in [[index]].

## Verify pass (2026-10-28)

- A whitelist saved as UTF-16LE (PowerShell 5.1 `Out-File`, Notepad
  "Unicode") is read and extended as UTF-16LE; UTF-8 bytes appended to it were
  garbage to Mole.
- A whitelist that cannot be written says so in the preview log
  (`MoleProtectFailed`) instead of failing silently.
- `ps_quote` doubles U+2018..U+201B too: PowerShell ends a single-quoted
  string at them, and the elevated command line is built from paths.
- Owner check still open: whether Mole 1.30 honours `whitelist.txt` (its docs
  describe it for later releases). The dry-run names groups, not paths, so the
  island cannot check it.

- The elevated step's exit code: an exit code the script sets is its word;
  a script without one fails on new `$Error` entries or a throw (with every
  stream redirected `$?` stays true). Tested with a real PowerShell on temp
  scripts (`the_elevated_step_fails_when_the_script_fails`).
