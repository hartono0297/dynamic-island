# Agent rules

## Every code change goes through the `build` workflow

I opt in to multi-agent orchestration for this project. Whenever I ask for a code
change (new feature, bug fix, refactor), run the `build` workflow
(`.claude/workflows/build.js`) with my request as `args`. Do not edit the code
yourself in the main session.

- Do not pick the mode yourself. The workflow's Plan step (Opus) decides
  between `full` and `lite`.
- The workflow merges passing tasks into `main` and pushes. This project uses a
  single branch (`main`), so that push is pre-approved. Deploy stays with me.
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
- Build: `cargo build --release --locked` → `target/release/dynamic-island.exe`
- Test: `cargo test --locked` (unit tests live inside `src/**` as `#[cfg(test)]` modules)
- Run locally: `cargo run --release`. The island is a single-instance UI at the
  top of the screen; stop any installed copy first (`Stop-Process -Name dynamic-island`).
- Release: push a `v*` tag; `.github/workflows/release.yml` builds and publishes.
