# Agent rules

## Models per phase (set in `.claude/workflows/build.js`)

| Phase | Model |
|---|---|
| Plan | Opus 5.5 (`claude-opus-5-5`) |
| Build (write the code) | Sonnet 5.5 (`claude-sonnet-5-5`) |
| Review and fix | Opus 5.5 (`claude-opus-5-5`) |
| Check | Sonnet 5.5 (`claude-sonnet-5-5`) |
| Ship | Sonnet 5.5 (`claude-sonnet-5-5`) |
| Finish (memory, README, push) | Sonnet 5.5 (`claude-sonnet-5-5`) |

- Spawning subagents (Agent tool) is allowed whenever it helps: parallel
  reading, independent tasks, blind reviewers, fixers. Set each agent's model
  from the table above for the phase it works in.
- Review and fix loop until the review comes back clean (up to 3 rounds).
- If a model cannot be selected (for example `model_not_found`), say so and
  ask before falling back to another model.

## Every code change goes through the `build` workflow

The workflow is the one from Gxl-WorkFlow (`D:\GitHub\Gxl-WorkFlow`,
`template/.claude/workflows/build.js`), copied unchanged with its model rules
(the table above). To update it, copy the template file again.

I opt in to multi-agent orchestration for this project. Whenever I ask for a code
change (new feature, bug fix, refactor), run the `build` workflow
(`.claude/workflows/build.js`) with my request as `args`. Do not edit the code
yourself in the main session.

- Do not weigh the tasks yourself. The workflow's Plan step (Opus) splits the
  request and marks each task `light` or `heavy`. Light tasks get build only
  (the builder and Ship run the tests). Heavy tasks get build, a blind review of
  a pinned commit (fixes land in the builder's worktree), and a check against a
  written checklist. What the checker cannot test lands in `manualChecks` for me.
- The workflow merges passing tasks into `main` locally. Once everything is
  ready (every task merged, reviewed, fixed and checked where required, tests
  green on `main`, nothing `incomplete`), its Finish step **always** updates the
  Obsidian memory and `README.md`, commits those docs, and pushes `main`. This
  project uses a single branch (`main`), so that push is pre-approved. Deploy
  stays with me.
- If anything is not ready, the workflow skips docs and push. Fix what it
  reports first; don't push half-finished work from the main session.
- For a long batch of runs, pass `args: {request, defer: true}` to build now,
  then run one verify pass with `args: {verify: [...]}` (the `deferred` list the
  runs returned) at the end. Deferred runs never push; the verify pass does the
  Finish step.
- When it finishes, report: which tasks were merged and their weight, what the
  Finish step wrote to memory and README, whether `main` was pushed,
  `manualChecks`, `incomplete`, and any open findings or failed checks.

## Always brainstorm before `build`

Agents inside the workflow cannot ask me anything, so every open question gets
settled first, in the main session, with me. Before every `build` run:

- Restate what you understood in a few lines, then ask about whatever is
  unclear: the goal, who uses it, the exact behaviour and edge cases, what must
  not change, and what "done" means. Ask a few focused questions at a time.
- For larger or open-ended work, propose 2-3 approaches with trade-offs and
  recommend one.
- Even for a small, clear request, restate it in one or two lines and wait for
  my OK.
- Only when I confirm, run `build` with the agreed request written out
  concretely, including the acceptance criteria.

## Superpowers must be off (check every session)

At the start of every session in this project, before the first task, check
whether the Superpowers plugin is enabled: skills named `superpowers:*` are
available in the session, or `enabledPlugins` in `~/.claude/settings.json`,
`.claude/settings.json` or `.claude/settings.local.json` has
`"superpowers@superpowers-marketplace": true` (on this machine the id is `superpowers@claude-plugins-official`).

If it is on, do not turn it off yourself. Tell me, explain why, and ask me to
confirm turning it off:

- Its skills overlap with `build` (orchestration, worktrees, finishing
  branches, code review), so two systems steer the same work: duplicate
  branches and worktrees, slower runs.
- Its "MUST" rules (such as brainstorming before any creative work) make
  workflow agents stop to ask questions nobody inside the workflow can answer.
- `using-superpowers` makes every agent check for skills before each reply,
  which costs tokens in every workflow agent.
- What is useful from it already lives here: brainstorming in the main session
  (section above), and test-first, root-cause fixing and proof-before-claims in
  the `build` prompts.

If I agree, ask whether to turn it off for all projects (`~/.claude/settings.json`)
or only this one (`.claude/settings.json`), set
`"superpowers@superpowers-marketplace": false` there, and tell me to restart
Claude Code. If I say keep it, don't ask again in this session.

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
    index. The `build` workflow does this in its Finish step.
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
- Demo and debug modes for visuals (no real user data, no media keys):
  `--demo`, `--demo-editor`, `--demo-cli`, `--demo-robot`, `--demo-devices`, `--settings`.
- Release: push a `v*` tag; `.github/workflows/release.yml` builds and publishes.
