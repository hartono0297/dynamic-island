---
tags: [handoff, v3.1, workflow]
date: 2026-10-28
---

# v3.1 handoff: where things stand and how to continue

Written so the work can continue on another PC. Plan: `docs/superpowers/plans/2026-10-27-v3.1.md`. Run order (not by number): 11, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 14, then the verify pass, then 10.

## Done and merged on `main` (built with `defer`, NOT yet reviewed or checked)

Run 11, 1, 2, 3, 4, 5, 6, 7 and 8 are on `main`; `cargo test --locked` gave 814 passed at the last merge (`c6c0555`).

## Left to do

| Step | What | Notes |
|---|---|---|
| Run 9 | Nine pages and the "Halaman panel" setting | Order: Calendar, Pomodoro, Reminders, Notes, Media, Clipboard, System, CLI, Shortcuts (the interim order already matches) |
| Run 12 | Rename to Vale and data migration | Repo rename on GitHub is the owner's job, not the agent's |
| Run 13 | Cat mascot and eared logo | Replaces the Bata robot; mascot name is still open |
| Run 14 | One-click Google sign-in | Client ID/secret are filled in by the owner; empty means the button is hidden |
| Verify pass | Review and check of the heavy runs | See the table below |
| Run 10 | i18n, screenshots, README, version 3.1.0 | Last, so screenshots show the final result |
| Push | `git push origin main` once, after Run 10 | Tag `v3.1.0` is the owner's job |

Still open for the owner, never decided by an agent: mascot name, repo licence (no `LICENSE` yet), website contact email, website domain, Google Client ID.

## How to run the workflow

`.claude/workflows/build.js` (see AGENTS.md and the repo README of Gxl-WorkFlow).

- Build now, verify later: `Workflow` with `args: {request: "...", defer: true}`. Heavy tasks are built and merged without review/check; the result lists them in `deferred`.
- Verify pass at the end: `args: {verify: [{id, brief, checklist, base, head}, ...]}` using the ranges below. It reviews each range on a pinned commit, fixes on a `fix/<id>` branch, checks against the checklist, merges the fix branches. `manualChecks` holds what no agent can test.
- Each run's request is the Run N text in the plan plus: "Baca docs/memory/index.md dan catatan terkait. Semua teks lewat src/i18n.rs (ID dan EN). Jangan push. Jangan membaca atau mengubah clipboard, file atau media asli pemilik; tes memakai folder sementara dan abstraksi palsu. Catatan memori di docs/memory."
- The workflow does not push. The main session pushes once at the end. If a subagent cannot push, that is the Claude Code auto-mode classifier; the main session can push (the user pre-approved it in AGENTS.md).
- If an agent dies on a usage limit, the builder's work is usually left uncommitted in its worktree: commit it as a WIP commit on the task branch (`git -C <worktree> add -A; git -C <worktree> commit`), remove the worktree with `git worktree remove` (no force), then launch a new run whose request says "continue from branch task/<id>, WIP commit <sha>, do not rewrite". Builders are now told to commit WIP often.
- Builders must branch from the LOCAL `main` (`git checkout -b task/<id> main`); worktrees may start from `origin/main`, which was stale during v3.1. After the push this is no longer a problem.
- Model names: `build.js` uses the aliases `opus` and `sonnet`. On the first PC these failed with "issue with the selected model" and the session script used the full ids `claude-opus-5-5` and `claude-sonnet-5-5` (constants `OPUS` and `SONNET` at the top of the file). Do the same if you see that error.
- Check agents must not click media buttons or do anything that interrupts playback; use the debug demo modes (`--demo`, `--demo-editor`, `--demo-cli`, `--demo-robot`, `--settings`) and `scripts/capture-screenshots.ps1`.

## Verify list: ranges merged without review or check

`base..head` on `main` (use `git log` for the commits). Heavy runs only.

| Run | id | base | head | What to check |
|---|---|---|---|---|
| 11 | media-pause-fix | 925887f | 42b578a | Press and release must pair; 540 ms guard after a hover open; commands go to the shown session; `--media-log`. Never click real media in a test |
| 1 | display-font | 42b578a | 43ca00a | DM Sans 600 upright; "100:00", song title, lyric, notch clock fit |
| 2 | lyric-notch-width | 43ca00a | 94a22d8 | Width 220 to 560, hysteresis, marquee; a line that changes mid-morph |
| 3 | cli-explorer-folder-menu | 94a22d8 | 53f7e6a | COM `IShellWindows`, folder menu, favourites, Win11 tabs |
| 4 | system-page | 53f7e6a | f41f563 | System page drawn; one-shot warnings; idle is cheap |
| 5 | deep-clean-own-mode | 2d0b5ec | 36519f1 | Cleaner "Pilih sendiri"; two-step confirm; deletes only in test folders |
| 6 | deep-clean-mole-mode | 36519f1 | 1cb9b14 | Mole mode; never runs Mole for real in tests; preview and confirm gate |
| 7 | clipboard-history | 67cc45a | a705a0f | History, DPAPI, password-manager exclusion; do not touch the real clipboard |
| 8 | screenshot-preview | a705a0f | c6c0555 | Match of file and clipboard image; no delete without the Hapus click; ignored preview keeps the file |

Heavy runs 9, 12, 13 and 14 get added here as they land.

## Owner checks that no agent could do

- Click media in the island: a click within 540 ms of the panel opening is ignored; a click after it pauses; press on one button and release on another does nothing.
- CLI keyboard focus in a fullscreen app; IME and emoji in the CLI input and the editors.
- The System page and the cleaner on the real machine (scan and delete, UAC prompt for Windows Update).
- Mole: version warning, preview without admin, whitelist for another Windows user. Mole's licence: its `windows` branch was reported to move to GPL-3.0 on 2026-10-11; location data came from the last MIT commit `500dfc4c`; the island only runs the user's Mole as a separate process. Verify this claim on github.com/tw93/mole.
- Clipboard: the optional hotkey (Ctrl+Alt+V or Win+Alt+V), IME in the search field.
- Screenshot: real PrtSc and Win+Shift+S, Snipping Tool file timing.
- Look at the screenshots that `scripts/capture-screenshots.ps1` makes (System, cleaner, Mole, clipboard, screenshot notice, long lyric, folder menu, DM Sans titles).

## Related

[[v3-studio-overview]], [[cli-panel-keyboard-focus]], [[bata-idle-power-budget]], [[media-click-guards]], [[cleaner]], [[mole-mode]], [[clipboard-history]], [[screenshot-preview]], [[screenshot-capture-script]]
