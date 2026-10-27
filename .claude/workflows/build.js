export const meta = {
  name: 'build',
  description: 'Opus plans and picks full or lite mode, builders work in parallel worktrees, blind review/fix until SHIP, check, then merge to main (the main session pushes)',
  whenToUse: 'Every code change. Pass the request as args, e.g. args: "add login page with email + password"',
  phases: [
    { title: 'Plan', detail: 'pick full/lite, split into independent tasks', model: 'opus' },
    { title: 'Build', detail: 'one builder per task, in its own worktree', model: 'opus' },
    { title: 'Review', detail: 'blind review (incl. edge cases) -> fix loop until SHIP' },
    { title: 'Check', detail: 'run tests + try it like a user', model: 'sonnet' },
    { title: 'Ship', detail: 'merge passing tasks into main, test once, push', model: 'sonnet' },
  ],
}

// full = large or risky work (review + check), lite = small/light work (build only; Ship still tests main)
const MODES = {
  full: { maxTasks: 4, fixRounds: 3, review: true, check: true, fixModel: 'opus', lowEffort: {} },
  lite: { maxTasks: 3, fixRounds: 2, review: false, check: false, fixModel: 'sonnet', lowEffort: { effort: 'low' } },
}

const PLAN = {
  type: 'object',
  properties: {
    mode: { type: 'string', enum: ['full', 'lite'] },
    reason: { type: 'string', description: 'one sentence: why this mode' },
    tasks: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string', description: 'short kebab-case id, used as branch name' },
          brief: { type: 'string', description: 'what to build, acceptance criteria, files likely touched' },
        },
        required: ['id', 'brief'],
      },
    },
  },
  required: ['mode', 'reason', 'tasks'],
}

const BUILD = {
  type: 'object',
  properties: {
    branch: { type: 'string' },
    summary: { type: 'string' },
  },
  required: ['branch', 'summary'],
}

const REVIEW = {
  type: 'object',
  properties: {
    verdict: { type: 'string', enum: ['SHIP', 'FIX'] },
    findings: { type: 'array', items: { type: 'string' } },
  },
  required: ['verdict', 'findings'],
}

const CHECK = {
  type: 'object',
  properties: {
    passed: { type: 'boolean' },
    notes: { type: 'string' },
  },
  required: ['passed', 'notes'],
}

const SHIP = {
  type: 'object',
  properties: {
    pushed: { type: 'boolean' },
    merged: { type: 'array', items: { type: 'string' }, description: 'branches merged into main' },
    notes: { type: 'string', description: 'skipped branches, test failures, or why nothing was pushed' },
  },
  required: ['pushed', 'merged', 'notes'],
}

// Tool rules every agent follows (see AGENTS.md)
const TOOLS =
  `\n\nTool rules:\n` +
  `- Shell commands go through RTK (a Bash hook rewrites them). Prefer rtk forms like "rtk git", "rtk test", "rtk grep"; use "rtk bypass <cmd>" only if output is unusable.\n` +
  `- For any library, framework, SDK, CLI or API: look up current docs with the Context7 MCP (resolve-library-id, then query-docs), matching the version the project uses. Do not rely on memory.\n` +
  `- Project memory is an Obsidian vault at docs/memory/ (start at docs/memory/index.md). Read the notes relevant to your work before starting.`

if (!args) throw new Error('Pass the request as args')

// 1. Plan — orchestrator role: pick the mode and split work, never code
phase('Plan')
const plan = await agent(
  `Read the project memory (docs/memory/index.md and the notes it links that relate to this request), then the codebase, and plan this request. Do not write code.\n\n` +
  `First pick a mode:\n` +
  `- "lite": a small or light change: few files, low risk, clear requirements, nothing in the core event/render/data paths. Lite skips the blind review and the live check, so only the builder and the final test run on main guard it.\n` +
  `- "full": anything else: medium or large changes, many files, core architecture (window/event/render pipeline, state, scheduling), user data or config formats, security, new features with UI, or unclear requirements. Full gets blind review and a live check.\n` +
  `When unsure, pick "full".\n\n` +
  `Then split the request into INDEPENDENT tasks that touch different files, so they can be built in parallel: ` +
  `at most ${MODES.lite.maxTasks} tasks for lite, ${MODES.full.maxTasks} for full. Prefer 1 task if the request is small.\n\n` +
  `Request: ${args}` + TOOLS,
  { model: 'opus', schema: PLAN }
)
const mode = MODES[plan.mode]
const tasks = plan.tasks.slice(0, mode.maxTasks)
log(`Mode: ${plan.mode} (${plan.reason})`)
log(`${tasks.length} task(s): ${tasks.map(t => t.id).join(', ')}`)
if (plan.tasks.length > tasks.length) log(`Dropped over-limit tasks: ${plan.tasks.slice(mode.maxTasks).map(t => t.id).join(', ')}`)

// 2-4. Each task flows through build -> review/fix -> check independently
const results = await pipeline(
  tasks,

  // Build
  task => agent(
    `Build this task. Create and commit on a new branch named "task/${task.id}". Run the relevant tests before finishing.\n\nBrief:\n${task.brief}` + TOOLS,
    { label: `build:${task.id}`, phase: 'Build', isolation: 'worktree', model: 'opus', schema: BUILD }
  ),

  // Blind review <-> fix loop
  async (build, task) => {
    if (!mode.review) return { ...build, verdict: 'SHIP', openFindings: [] }
    let review
    for (let round = 1; round <= mode.fixRounds; round++) {
      review = await agent(
        `You are a blind reviewer. You only get the brief and the diff of branch "${build.branch}" (git diff against its base). Do not read the builder's notes. Check correctness against the brief AND try to break it with edge cases (empty input, errors, boundaries). Return SHIP only if there is nothing worth fixing.\n\nBrief:\n${task.brief}` + TOOLS,
        { label: `review:${task.id}#${round}`, phase: 'Review', model: 'opus', schema: REVIEW }
      )
      if (review.verdict === 'SHIP') break
      await agent(
        `Check out branch "${build.branch}", fix these findings, and commit on the same branch:\n- ${review.findings.join('\n- ')}` + TOOLS,
        { label: `fix:${task.id}#${round}`, phase: 'Review', isolation: 'worktree', model: mode.fixModel }
      )
    }
    if (review.verdict !== 'SHIP') log(`${task.id}: still FIX after ${mode.fixRounds} rounds`)
    return { ...build, verdict: review.verdict, openFindings: review.verdict === 'SHIP' ? [] : review.findings }
  },

  // Check — tests + live run in one agent
  async (reviewed, task) => {
    if (!mode.check) return { id: task.id, ...reviewed, check: { passed: true, notes: 'skipped in lite mode: the builder ran the tests and Ship tests main' } }
    const check = await agent(
      `Check out branch "${reviewed.branch}". Run the full test suite, then start the app locally and use the feature like a real user. Report whether it works.\n\nBrief:\n${task.brief}` + TOOLS,
      { label: `check:${task.id}`, phase: 'Check', isolation: 'worktree', schema: CHECK, model: 'sonnet', ...mode.lowEffort }
    )
    return { id: task.id, ...reviewed, check }
  }
)

// Ship — single-branch flow: task branches are temporary, everything lands on main; the main session pushes once all work is done.
// Barrier on purpose: merging needs all finished tasks together.
const done = results.filter(Boolean)
const ready = done.filter(r => r.verdict === 'SHIP' && r.check.passed)
const held = done.filter(r => !ready.includes(r))
if (held.length) log(`Not shipped: ${held.map(r => r.id).join(', ')}`)

let ship = { pushed: false, merged: [], notes: 'Nothing passed review + check' }
if (ready.length) {
  phase('Ship')
  ship = await agent(
    `Ship these finished task branches to main, in this order: ${ready.map(r => r.branch).join(', ')}.\n` +
    `1. If the working tree on main has uncommitted changes, STOP and report — do not stash or discard them.\n` +
    `2. Check out main and pull. Merge each branch into main. If a merge conflict is not trivial, abort that merge, skip the branch, and report it.\n` +
    `3. Run the full test suite once on main. If it fails, do NOT push — report the failures.\n` +
    `4. If green, update project memory in docs/memory/ (Obsidian vault): add or update one note per decision, gotcha or convention worth remembering from this work ` +
    `(frontmatter with tags + today's date, [[wikilinks]] to related notes), link new notes from docs/memory/index.md, and commit as "docs(memory): ...". ` +
    `Skip if nothing new is worth remembering. Do not duplicate what the code or git history already says.\n` +
    `5. Do NOT push; the main session pushes after all work is finished. Delete the merged task branches (local + their worktrees).\n\n` +
    `Request: ${args}\nTasks:\n${ready.map(r => `- ${r.id}: ${r.summary}`).join('\n')}` + TOOLS,
    { label: 'ship:main', phase: 'Ship', model: 'sonnet', schema: SHIP, ...mode.lowEffort }
  )
}

return {
  mode: plan.mode,
  modeReason: plan.reason,
  pushed: ship.pushed,
  shipNotes: ship.notes,
  tasks: done.map(r => ({
    task: r.id,
    branch: r.branch,
    verdict: r.verdict,
    testsPassed: r.check.passed,
    merged: ship.merged.includes(r.branch),
    summary: r.summary,
    openFindings: r.openFindings,
    checkNotes: r.check.notes,
  })),
}
