# Agent rules

> **Temporarily disabled (2026-10-28).** The `build` workflow is switched off for
> this project: make code changes directly in the main session, without
> multi-agent orchestration. The workflow file was moved to
> `.claude/workflows-disabled/build.js`. To switch it back on, move it to
> `.claude/workflows/build.js` and delete this note; the section below then
> applies again.

## Models per phase (applies now, with or without the workflow)

| Phase | Model |
|---|---|
| Plan | Opus 5.5 (`claude-opus-5-5`) |
| Execute (write the code) | Sonnet 5.5 (`claude-sonnet-5-5`) |
| Review, fix and check | Opus 5.5 (`claude-opus-5-5`) |

- Spawning subagents (Agent tool) is allowed whenever it helps: parallel
  reading, independent tasks, blind reviewers, fixers. Set each agent's model
  from the table above for the phase it works in.
- Review, fix and check loop until the review comes back clean.
- If a model cannot be selected (for example `model_not_found`), say so and
  ask before falling back to another model.

## Every code change goes through the `build` workflow (disabled, see above)

I opt in to multi-agent orchestration for this project. Whenever I ask for a code
change (new feature, bug fix, refactor), run the `build` workflow
(`.claude/workflows/build.js`) with my request as `args`. Do not edit the code
yourself in the main session.

- Do not pick the mode yourself. The workflow's Plan step (Opus) decides
  between `full` and `lite`.
- `lite` (small or light tasks) skips the blind review and the live check: the
  builder runs the tests, and Ship tests `main` before pushing. Only `full`
  (medium or large, or core/risky work) gets review and check. The rule is in
  `.claude/workflows/build.js` (`MODES.*.review` / `.check`).
- The workflow merges passing tasks into `main` locally and does not push. The
  main session pushes `main` once all the requested work is finished (for a
  batch of runs: after the last run and after the deferred review/check pass).
  This project uses a single branch (`main`), so that push is pre-approved.
  Deploy stays with me.
- Each task is marked light or heavy by the Plan step. Light tasks get build
  only (the builder and Ship run the tests). Heavy tasks get build, a blind
  review of a pinned commit (fixes land in the builder's worktree), and a check
  against a written checklist; anything the checker cannot test lands in
  `manualChecks` for me.
- For a long batch of runs, pass `args: {request, defer: true}` to build now,
  then run one verify pass with `args: {verify: [...]}` (the `deferred` list the
  runs returned) at the end, before the push.
- When it finishes, report: mode and why, which tasks were merged, whether
  `main` was pushed, and any open findings or failed checks.

## Preflight: once per project

Runs only the very first time Claude works in this project. If
`.claude/preflight.done` exists, skip this section entirely.

Otherwise, before the first task, check that the three tools below are in
place and install whatever is missing. Then create `.claude/preflight.done`
(today's date + what was installed), commit it, and report what you installed.

| Tool | Check | If missing, install |
|---|---|---|
| RTK | `rtk --version`, and `~/.claude/settings.json` has a Bash `PreToolUse` hook running `rtk hook claude` | Windows: `winget install --id rtk-ai.rtk -e`. Then `rtk init -g --auto-patch` |
| Context7 | `claude mcp list` shows `context7` | `claude mcp add --scope user --transport http context7 https://mcp.context7.com/mcp` |
| Obsidian vault | `docs/memory/index.md` exists in this project | Create it with the Decisions / Gotchas / Conventions sections and commit it. If the Obsidian app is missing (Windows: `winget list --id Obsidian.Obsidian`), install it with `winget install --id Obsidian.Obsidian -e` |

- A newly added MCP server only loads in a new session. If Context7 was just
  installed, tell me to restart Claude Code before running `build`.
- Opening `docs/` as a vault in the Obsidian app is my step; remind me if it's new.
- To run it again (e.g. on a new machine), I delete `.claude/preflight.done`.

## Tools (main session and every workflow agent)

- **RTK**: shell output goes through RTK to save tokens. The global Bash hook
  rewrites commands. Prefer `rtk git`, `rtk test`, `rtk grep`, etc., and use
  `rtk bypass <cmd>` only when a result is unusable.
- **Context7**: for any library, framework, SDK, CLI or API, look up current
  docs with the Context7 MCP (`resolve-library-id`, then `query-docs`) before
  answering or writing code. Match the version the project uses.
- **Obsidian memory**: project memory lives in the Obsidian vault at `docs/`.
  Notes go in `docs/memory/`, and the index is `docs/memory/index.md`.
  - At the start of a task, read the index and the notes relevant to it.
  - After work that taught something worth keeping (a decision, gotcha,
    convention), add or update one note per fact. Give it frontmatter with
    `tags` and `date`, `[[wikilinks]]` to related notes, and a link from the
    index. The `build` workflow does this in its Ship step.
  - Don't save what the code or git history already records. Update an
    existing note instead of creating a duplicate.

## When NOT to run it

- Questions, explanations, reading code, or planning without changes.
- Edits that are not code: docs, README, comments, config values, typos.
- When I explicitly say "do it directly" / "tanpa workflow".

## Project commands

Rust (edition 2024), Windows 11 x64 only. Needs the stable Rust toolchain with
the MSVC target (`rustup`, `cargo`); CI builds with `cargo build --release --locked`.

- Install: `rustup toolchain install stable --profile minimal` (deps come via cargo)
- Build: `cargo build --release --locked` → `target/release/vale.exe`
- Test: `cargo test --locked` (unit tests live inside `src/**` as `#[cfg(test)]` modules)
- Run locally: `cargo run --release`. The island is a single-instance UI at the
  top of the screen; stop any installed copy first (`Stop-Process -Name vale, dynamic-island`).
- Release: push a `v*` tag; `.github/workflows/release.yml` builds and publishes.
