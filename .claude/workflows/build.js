export const meta = {
  name: 'build',
  description: 'Plan (per-task light/heavy) -> build in worktrees -> heavy tasks: blind review/fix on a pinned commit, then check against a written checklist -> merge to main locally (the main session pushes)',
  whenToUse: 'Every code change. args: a request string, or {request, defer: true} to build now and verify heavy tasks later, or {verify: [...]} for the deferred review/check pass',
  phases: [
    { title: 'Plan', detail: 'split into tasks, mark each light or heavy, write a checklist for heavy ones', model: 'opus' },
    { title: 'Build', detail: 'one builder per task, in its own worktree', model: 'sonnet' },
    { title: 'Review', detail: 'heavy only: blind review of a pinned commit, fixes land in the builder worktree', model: 'opus' },
    { title: 'Check', detail: 'heavy only: tests + the written checklist', model: 'sonnet' },
    { title: 'Ship', detail: 'merge passing tasks into local main, test once; no push', model: 'sonnet' },
  ],
}

const OPUS = 'opus'
const SONNET = 'sonnet'
const MAX_TASKS = 4
const FIX_ROUNDS = 3

const PLAN = {
  type: 'object',
  properties: {
    reason: { type: 'string', description: 'one sentence on how the work was split and weighed' },
    tasks: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string', description: 'short kebab-case id, used as branch name' },
          weight: { type: 'string', enum: ['light', 'heavy'] },
          brief: { type: 'string', description: 'what to build, acceptance criteria, files likely touched. Notes and docs for this change belong in this same task.' },
          checklist: {
            type: 'array',
            items: { type: 'string' },
            description: 'heavy tasks only: concrete things a checker must verify (what to open or run, what to compare with the design, what is off limits)',
          },
        },
        required: ['id', 'weight', 'brief'],
      },
    },
  },
  required: ['reason', 'tasks'],
}

const BUILD = {
  type: 'object',
  properties: {
    branch: { type: 'string' },
    sha: { type: 'string', description: 'git rev-parse HEAD of the final commit on the branch' },
    worktree: { type: 'string', description: 'absolute path of this worktree (git rev-parse --show-toplevel)' },
    summary: { type: 'string' },
  },
  required: ['branch', 'sha', 'worktree', 'summary'],
}

const REVIEW = {
  type: 'object',
  properties: {
    reviewed_sha: { type: 'string', description: 'the commit you actually reviewed (git rev-parse HEAD or the end of the range)' },
    verdict: { type: 'string', enum: ['SHIP', 'FIX'] },
    findings: { type: 'array', items: { type: 'string' } },
  },
  required: ['reviewed_sha', 'verdict', 'findings'],
}

const FIX = {
  type: 'object',
  properties: {
    sha: { type: 'string', description: 'git rev-parse HEAD after your commit (unchanged if you committed nothing)' },
    branch: { type: 'string' },
    worktree: { type: 'string' },
    summary: { type: 'string' },
  },
  required: ['sha', 'summary'],
}

const CHECK = {
  type: 'object',
  properties: {
    items: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          item: { type: 'string' },
          status: { type: 'string', enum: ['pass', 'fail', 'cannot_test'] },
          note: { type: 'string' },
        },
        required: ['item', 'status'],
      },
    },
    notes: { type: 'string' },
  },
  required: ['items', 'notes'],
}

const SHIP = {
  type: 'object',
  properties: {
    merged: { type: 'array', items: { type: 'string' }, description: 'branches merged into main' },
    base: { type: 'string', description: 'main sha before the first merge' },
    head: { type: 'string', description: 'main sha after the last merge' },
    tests_passed: { type: 'boolean' },
    notes: { type: 'string', description: 'skipped branches, test failures, anything the main session must know' },
  },
  required: ['merged', 'base', 'head', 'tests_passed', 'notes'],
}

// Rules every agent follows (see AGENTS.md)
const TOOLS =
  `\n\nTool rules:\n` +
  `- Shell commands go through RTK (a Bash hook rewrites them). Prefer rtk forms like "rtk git", "rtk test", "rtk grep"; use "rtk bypass <cmd>" only if output is unusable.\n` +
  `- For any library, framework, SDK, CLI or API: look up current docs with the Context7 MCP (resolve-library-id, then query-docs), matching the version the project uses. Do not rely on memory.\n` +
  `- Project memory is an Obsidian vault at docs/memory/ (start at docs/memory/index.md). Read the notes relevant to your work before starting.\n` +
  `- Never do anything that could interrupt the owner's other apps (media playback, calls, running builds). For visuals use the project's own demo or debug modes if it has any, never anything that touches real user data.\n` +
  `- Never push to origin and never force-delete branches or worktrees. The main session does that.`

if (!args) throw new Error('Pass the request as args')
const input = typeof args === 'string' ? { request: args } : args
const incomplete = []

// agent() that never throws: a failed or skipped agent becomes null plus a note
async function safe(label, prompt, opts) {
  try {
    const out = await agent(prompt, { label, ...opts })
    if (out == null) incomplete.push(`${label}: no result`)
    return out
  } catch (e) {
    incomplete.push(`${label}: ${e && e.message ? e.message : e}`)
    return null
  }
}

function reviewPrompt(subject, brief, sha) {
  return (
    `You are a blind reviewer. ${subject}. Do not read any builder notes. Check correctness against the brief AND try to break it with edge cases (empty input, errors, boundaries, old data and config files). ` +
    `First run git rev-parse on the end of what you review and return it as reviewed_sha; it must be ${sha}. Return SHIP only if there is nothing worth fixing.\n\nBrief:\n${brief}`
  )
}

async function reviewAt(label, subject, brief, sha) {
  for (let attempt = 1; attempt <= 2; attempt++) {
    const r = await safe(label, reviewPrompt(subject, brief, sha) + TOOLS, { phase: 'Review', model: OPUS, schema: REVIEW })
    if (!r) return null
    if (r.reviewed_sha && r.reviewed_sha.startsWith(sha.slice(0, 7))) return r
    log(`${label}: reviewer looked at ${r.reviewed_sha}, not ${sha}; asking again`)
  }
  incomplete.push(`${label}: the reviewer never looked at ${sha}`)
  return null
}

function checkPrompt(where, sha, task) {
  const list = (task.checklist && task.checklist.length ? task.checklist : ['The feature in the brief works as described']).map((c, i) => `${i + 1}. ${c}`).join('\n')
  return (
    `${where}\nConfirm that git rev-parse HEAD is ${sha}; if not, say so in notes and stop. Do not modify tracked files and do not commit.\n` +
    `Run the full test suite (the project's test command, see 'Project commands' in AGENTS.md) and the project's build command. Then go through this checklist item by item and report each as pass, fail, or cannot_test. ` +
    `Use cannot_test (with the reason) for anything that needs an action you must not take or a setup you do not have (real third-party accounts, real hardware, other apps' playback, anything that needs a human), instead of guessing. Use the project's demo or debug modes for visuals and compare with the design files named in the brief.\n\nChecklist:\n${list}\n\nBrief:\n${task.brief}` +
    TOOLS
  )
}

function checkResult(c) {
  if (!c) return { passed: false, skipped: false, items: [], notes: 'the check agent failed' }
  const failed = c.items.filter(i => i.status === 'fail')
  return { passed: failed.length === 0, skipped: false, items: c.items, notes: c.notes }
}

async function ship(label, ready, request) {
  return safe(
    label,
    `Ship these finished task branches to main, in this order: ${ready.map(r => r.branch).join(', ')}.\n` +
    `1. If the working tree on main has uncommitted changes (other than the untracked .claude/worktrees/ folder), STOP and report; do not stash or discard them.\n` +
    `2. Check out main. Note its sha as base. Merge each branch into main with --no-ff. If a merge conflict is not trivial, abort that merge, skip the branch, and report it. A conflict in docs/memory/index.md is trivial: keep both sides' lines.\n` +
    `3. Run the full test suite once on main (the project's test command, see 'Project commands' in AGENTS.md). If it fails, report the failures. Note main's sha as head.\n` +
    `4. If green, update project memory in docs/memory/ (Obsidian vault) only for decisions or gotchas not already written by the branches: one note each, frontmatter with tags and today's date, [[wikilinks]], linked from docs/memory/index.md; commit as "docs(memory): ...". Skip if nothing new.\n` +
    `5. Do NOT push; the main session pushes after all work is finished. Do not delete branches or worktrees.\n\n` +
    `Request: ${request}\nTasks:\n${ready.map(r => `- ${r.id}: ${r.summary}`).join('\n')}` + TOOLS,
    { phase: 'Ship', model: SONNET, schema: SHIP }
  )
}

function manualChecks(rows) {
  return rows.flatMap(r => ((r.check && r.check.items) || []).filter(i => i.status === 'cannot_test').map(i => `${r.id}: ${i.item}${i.note ? ' (' + i.note + ')' : ''}`))
}

// ---------------------------------------------------------------- verify pass
// args.verify = [{id, brief, checklist, base, head}]: review + check heavy work that was merged earlier with defer.
if (input.verify) {
  const items = input.verify
  const results = await pipeline(
    items,

    async it => {
      let sha = it.head
      let branch = null
      let worktree = null
      let review = null
      for (let attempt = 0; attempt <= FIX_ROUNDS; attempt++) {
        review = await reviewAt(
          `review:${it.id}#${attempt + 1}`,
          `You get the brief and the diff git diff ${it.base}..${sha} (read it from the main repository; if a fix branch exists use ${branch || 'none'})`,
          it.brief,
          sha
        )
        if (!review) return { id: it.id, state: 'review-failed', verdict: 'UNKNOWN', openFindings: [] }
        if (review.verdict === 'SHIP' || attempt === FIX_ROUNDS) break
        const fix = await safe(
          `fix:${it.id}#${attempt + 1}`,
          branch
            ? `Work ONLY in the worktree ${worktree} (cd there; use git -C ${worktree}). Fix these findings and commit on branch ${branch}:\n- ${review.findings.join('\n- ')}` + TOOLS
            : `Create a new branch "fix/${it.id}" from the current main (HEAD), fix these findings in the code that was added by ${it.base}..${it.head}, run the project's test command, commit, and report sha, branch and worktree path:\n- ${review.findings.join('\n- ')}` + TOOLS,
          { phase: 'Review', isolation: branch ? undefined : 'worktree', model: OPUS, schema: FIX }
        )
        if (!fix || fix.sha === sha) { log(`${it.id}: the fix produced no new commit`); break }
        sha = fix.sha
        branch = fix.branch || branch
        worktree = fix.worktree || worktree
      }
      return { id: it.id, state: 'reviewed', verdict: review.verdict, openFindings: review.verdict === 'SHIP' ? [] : review.findings, sha, branch, worktree, task: it }
    },

    async r => {
      if (r.state !== 'reviewed' || r.verdict !== 'SHIP') return { ...r, check: { passed: false, skipped: true, items: [], notes: 'not checked: review did not end in SHIP' } }
      const where = r.worktree ? `Work in the worktree ${r.worktree} (cd there).` : `Check out ${r.sha} detached in your own worktree.`
      const c = await safe(`check:${r.id}`, checkPrompt(where, r.sha, r.task), { phase: 'Check', isolation: r.worktree ? undefined : 'worktree', model: SONNET, schema: CHECK })
      return { ...r, check: checkResult(c) }
    }
  )

  const rows = results.filter(Boolean)
  const fixed = rows.filter(r => r.branch && r.verdict === 'SHIP' && r.check.passed)
  let shipped = { merged: [], tests_passed: true, notes: 'no fix branches to merge' }
  if (fixed.length) {
    phase('Ship')
    shipped = (await ship('ship:verify', fixed.map(r => ({ ...r, summary: `fixes for ${r.id}` })), 'deferred verification fixes')) || { merged: [], tests_passed: false, notes: 'ship agent failed' }
  }
  return {
    pushed: false,
    shipNotes: shipped.notes,
    items: rows.map(r => ({ id: r.id, verdict: r.verdict, openFindings: r.openFindings, fixBranch: r.branch || null, checkPassed: r.check.passed, checkItems: r.check.items, checkNotes: r.check.notes })),
    manualChecks: manualChecks(rows),
    incomplete,
  }
}

// ---------------------------------------------------------------- normal pass
phase('Plan')
const plan = await safe(
  'plan',
  `Read the project memory (docs/memory/index.md and the notes it links that relate to this request), then the codebase, and plan this request. Do not write code.\n\n` +
  `Split the request into INDEPENDENT tasks that touch different files, so they can be built in parallel (at most ${MAX_TASKS}; prefer 1 task if the request is small). Notes and docs for a change go in the same task as its code, never in a separate task. Builders are Sonnet, so write each brief concretely: exact behaviour, acceptance criteria, files to touch.\n\n` +
  `Mark each task with a weight:\n` +
  `- "light": small, low-risk, few files, clear requirements, nothing in the core state, rendering or data paths. Light tasks are built and tested only; there is no review and no live check.\n` +
  `- "heavy": medium or large; core architecture (state, scheduling, rendering, concurrency, input focus); user data, config or file formats; security; platform APIs or threading; new features with UI; unclear requirements; or anything you are not sure is safe. Heavy tasks get a blind review and a check.\n` +
  `When unsure, pick "heavy".\n\n` +
  `For every heavy task also write "checklist": 3 to 10 concrete things a checker must verify (which demo mode to open, what to compare with which design file, which flows to exercise, which old data to load) and say what must NOT be touched (for example media buttons).\n\n` +
  `Request: ${input.request}` + TOOLS,
  { phase: 'Plan', model: OPUS, schema: PLAN }
)
if (!plan || !plan.tasks || !plan.tasks.length) throw new Error('Plan failed: ' + incomplete.join('; '))
const tasks = plan.tasks.slice(0, MAX_TASKS)
log(`${tasks.length} task(s): ${tasks.map(t => `${t.id} [${t.weight}]`).join(', ')} (${plan.reason})${input.defer ? '; defer: heavy tasks are verified later' : ''}`)
if (plan.tasks.length > tasks.length) log(`Dropped over-limit tasks: ${plan.tasks.slice(MAX_TASKS).map(t => t.id).join(', ')}`)

const results = await pipeline(
  tasks,

  // Build
  async task => {
    const b = await safe(
      `build:${task.id}`,
      `Build this task. FIRST: your worktree may start from an older commit (the last pushed origin/main), not from the local main that holds all finished work. Before anything else run git log -1 --format=%H main and create your branch from that commit: git checkout -b "task/${task.id}" main. Verify with git merge-base --is-ancestor main HEAD. Then work and commit on that branch. Commit early and often (a small WIP commit every time something compiles or a milestone is done) so the work survives if you are cut off; amend or squash at the end is optional. Run the relevant tests (the project's test command, see 'Project commands' in AGENTS.md) before finishing and make sure the build is warning-free. Report the branch, the final commit sha (git rev-parse HEAD) and this worktree's absolute path (git rev-parse --show-toplevel).\n\nBrief:\n${task.brief}` + TOOLS,
      { phase: 'Build', isolation: 'worktree', model: SONNET, schema: BUILD }
    )
    return b ? { ...b, id: task.id, weight: task.weight, state: 'built' } : { id: task.id, weight: task.weight, state: 'build-failed' }
  },

  // Blind review <-> fix loop, on a pinned commit, heavy tasks only
  async (r, task) => {
    if (r.state !== 'built') return r
    if (task.weight !== 'heavy') return { ...r, verdict: 'SKIPPED', openFindings: [] }
    if (input.defer) return { ...r, verdict: 'DEFERRED', openFindings: [] }
    let sha = r.sha
    let review = null
    for (let attempt = 0; attempt <= FIX_ROUNDS; attempt++) {
      review = await reviewAt(`review:${task.id}#${attempt + 1}`, `You get the brief and the diff of branch "${r.branch}" at commit ${sha} (git diff against its base)`, task.brief, sha)
      if (!review) return { ...r, verdict: 'UNKNOWN', openFindings: [], note: 'review failed' }
      if (review.verdict === 'SHIP' || attempt === FIX_ROUNDS) break
      const fix = await safe(
        `fix:${task.id}#${attempt + 1}`,
        `Work ONLY in the builder's worktree ${r.worktree} (cd there; use git -C ${r.worktree}); it has branch "${r.branch}" checked out. Do not create another branch and do not check the branch out anywhere else. Fix these findings, run the project's test command, and commit on that branch. Return the new sha (git rev-parse HEAD).\n- ${review.findings.join('\n- ')}` + TOOLS,
        { phase: 'Review', model: OPUS, schema: FIX }
      )
      if (!fix || fix.sha === sha) { log(`${task.id}: the fix produced no new commit`); break }
      sha = fix.sha
    }
    if (review.verdict !== 'SHIP') log(`${task.id}: still FIX after ${FIX_ROUNDS} rounds`)
    return { ...r, sha, verdict: review.verdict, openFindings: review.verdict === 'SHIP' ? [] : review.findings }
  },

  // Check against the written checklist, heavy tasks only, in the builder's worktree
  async (r, task) => {
    if (r.state !== 'built') return r
    if (r.verdict === 'SKIPPED' || r.verdict === 'DEFERRED') return { ...r, check: { passed: true, skipped: true, items: [], notes: r.verdict === 'SKIPPED' ? 'light task: the builder ran the tests and Ship tests main' : 'deferred to the verify pass' } }
    if (r.verdict !== 'SHIP') return { ...r, check: { passed: false, skipped: true, items: [], notes: 'not checked: review did not end in SHIP' } }
    const c = await safe(`check:${task.id}`, checkPrompt(`Work in the builder's worktree ${r.worktree} (cd there); branch "${r.branch}".`, r.sha, task), { phase: 'Check', model: SONNET, schema: CHECK })
    return { ...r, check: checkResult(c) }
  }
)

// Ship: barrier on purpose, merging needs every finished task together.
const done = results.filter(Boolean)
const ready = done.filter(r => r.state === 'built' && ['SHIP', 'SKIPPED', 'DEFERRED'].includes(r.verdict) && r.check.passed)
const held = done.filter(r => !ready.includes(r))
if (held.length) log(`Not shipped: ${held.map(r => r.id).join(', ')}`)

let shipped = { merged: [], base: '', head: '', tests_passed: false, notes: 'Nothing passed' }
if (ready.length) {
  phase('Ship')
  shipped = (await ship('ship:main', ready, input.request)) || { merged: [], base: '', head: '', tests_passed: false, notes: 'the ship agent failed' }
}

const byId = Object.fromEntries(tasks.map(t => [t.id, t]))
return {
  pushed: false,
  shipNotes: shipped.notes,
  testsPassedOnMain: shipped.tests_passed,
  base: shipped.base,
  head: shipped.head,
  tasks: done.map(r => ({
    task: r.id,
    weight: r.weight,
    state: r.state,
    branch: r.branch || null,
    sha: r.sha || null,
    worktree: r.worktree || null,
    verdict: r.verdict || null,
    merged: shipped.merged.includes(r.branch),
    summary: r.summary || null,
    openFindings: r.openFindings || [],
    check: r.check || null,
  })),
  // heavy work built with defer: feed this to a later run as args.verify
  deferred: input.defer
    ? done.filter(r => r.verdict === 'DEFERRED' && shipped.merged.includes(r.branch)).map(r => ({ id: r.id, brief: byId[r.id].brief, checklist: byId[r.id].checklist || [], base: shipped.base, head: shipped.head }))
    : [],
  manualChecks: manualChecks(done),
  incomplete,
}
