export const meta = {
  name: 'adversarial-round',
  description:
    'One adversarial hardening round: enumerate a plane, hunt it with N narrow lenses in parallel, close the findings, then independently verify the fixes are load-bearing. Fully parameterized via args — carries no repo-specific paths.',
  phases: [
    { title: 'Map', detail: 'enumerate the surface into an inventory' },
    { title: 'Hunt', detail: 'N narrow lenses in parallel, read-only' },
    { title: 'Close', detail: 'adjudicate + fix every copy + mutation-verify' },
    { title: 'Verify', detail: 'independent adversary proves the guards bite' },
  ],
}

// ---------------------------------------------------------------------------
// CONFIG comes entirely from `args`. Nothing about any specific repo lives in
// this file — that was the single biggest defect of the original antidrift-*
// scripts (10 scripts, all dead the moment you point them at another tree).
//
// Shape of args (all fields optional except lenses + testCommands):
// {
//   round: 7,                          // label only
//   baseline: "green at <commit>; ...",// what "clean" means right now
//   threatModel: "PRIZE: read X / overwrite Y ...",
//   surface: "the plane under attack this round, in prose",
//   mapPrompt: "how to enumerate the surface into an inventory",
//   lenses: [{ key, task }, ...],      // the narrow hunters — the yield lever
//   testCommands: "cd a && go test ./... ; cd b && npx vitest run ...",
//   knownGaps: ["title", ...],         // do-not-re-report, passed by value (NOT /tmp)
//   isolation: false,                  // true => each hunter in its own worktree
// }
// ---------------------------------------------------------------------------
// args may arrive as a parsed object OR, when passed to a named workflow, as a
// JSON string — tolerate both rather than silently throwing "lenses required".
let cfg = args || {}
if (typeof cfg === 'string') {
  try {
    cfg = JSON.parse(cfg)
  } catch (e) {
    throw new Error('adversarial-round: args was a string but not valid JSON: ' + e.message)
  }
}
const ROUND = cfg.round ?? 1
const LENSES = cfg.lenses || []
if (!LENSES.length) {
  throw new Error('adversarial-round: args.lenses is required (at least one { key, task })')
}
const TESTS =
  cfg.testCommands ||
  'RUN THE PROJECT TEST SUITES HERE — none were supplied in args.testCommands.'
const KNOWN = cfg.knownGaps || []

// Shared context every agent sees. Assembled from args, never hardcoded.
const CONTEXT = `
ROUND ${ROUND} of an adversarial hardening pass.

BASELINE (what "clean" means right now):
${cfg.baseline || '(none supplied — establish the green baseline yourself before you start.)'}

SURFACE UNDER ATTACK THIS ROUND:
${cfg.surface || '(none supplied.)'}

THREAT MODEL:
${cfg.threatModel || '(none supplied — reason about what an attacker gains from each defect.)'}

ALREADY FOUND AND FIXED — do NOT re-report these (${KNOWN.length}):
${KNOWN.length ? KNOWN.map((k) => `  - ${k}`).join('\n') : '  (none yet)'}

STANDING RULES (these are the method, not decoration):
- Evidence is A COMMAND AND ITS OUTPUT. An inspection-only claim ("I read the code and...")
  is rejected. If you cannot make it fail, you have not found it.
- A guard that kills no mutant does not exist. Prove every claim by breaking the thing,
  watching the NAMED test go red, and restoring. "The suite is green" proves nothing — it
  was green the whole time.
- The same concept is reimplemented per stack here. A fix to one copy that is missing from
  the twin that actually runs in the default configuration is not a fix. When you name a
  twin, change BOTH.
- An empty result is a SUCCESS. This pass exists to reach convergence, not to hit a quota.
  If your surface is genuinely clean, say so and say exactly what you tried.
- Leaving a TRACKED file modified is a failure of a HUNTER (hunters are read-only). Verify
  with \`git status --short\` before you return. Scratch files are fine; delete them.
- Watch disk: if /tmp is a quota tmpfs, a full one stops the shell from exec'ing at all and
  looks like every command failing with no output, not like a test failure. Clean up scratch.
`

// ---- schemas: ONE reconciled set (the original family had 3 close + 2 hunt) ----

const FINDING = {
  type: 'object',
  additionalProperties: false,
  required: ['title', 'input', 'expected', 'actual', 'evidence', 'severity'],
  properties: {
    title: { type: 'string', description: 'stable, specific — used for dedup across rounds' },
    input: { type: 'string' },
    expected: { type: 'string' },
    actual: { type: 'string' },
    evidence: { type: 'string', description: 'the command run AND its output. no output => rejected' },
    severity: { type: 'string', enum: ['critical', 'high', 'medium', 'low'] },
  },
}

const HUNT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['newGaps', 'triedAndClean', 'detail'],
  properties: {
    newGaps: { type: 'array', items: FINDING },
    triedAndClean: {
      type: 'string',
      description: 'what you attacked that held — a clean surface must still show its work',
    },
    detail: { type: 'string' },
  },
}

const CLOSE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['real', 'dismissed', 'filesChanged', 'mutationProof', 'testsRun', 'weakenedAnyCase'],
  properties: {
    real: { type: 'array', items: { type: 'string' }, description: 'titles fixed' },
    dismissed: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['title', 'why'],
        properties: { title: { type: 'string' }, why: { type: 'string' } },
      },
    },
    filesChanged: { type: 'array', items: { type: 'string' } },
    mutationProof: {
      type: 'string',
      description:
        'per fix: which case/test you broke, that it went red, that it went green on restore. NOT "suite passes".',
    },
    testsRun: { type: 'string', description: 'the exact commands and their pass/fail counts' },
    weakenedAnyCase: {
      type: 'string',
      description:
        'if you made ANY existing case/assertion less strict to go green, name it and why. ' +
        'Verify this PROGRAMMATICALLY by diffing the case-name->expectation map against ' +
        'git show HEAD:<path>, do not merely assert "I did not".',
    },
  },
}

const VERIFY_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['suitesGreen', 'counts', 'stillDecoration', 'catchesOriginalDefect', 'newGuardFails', 'detail'],
  properties: {
    suitesGreen: { type: 'boolean' },
    counts: { type: 'string' },
    stillDecoration: {
      type: 'array',
      items: { type: 'string' },
      description:
        'guards/tests added or touched this round that are DELETABLE with every suite still ' +
        'green — i.e. assert nothing. An honest entry here is worth more than a clean report.',
    },
    catchesOriginalDefect: {
      type: 'string',
      description:
        'for each fix: re-introduce the ORIGINAL bug and confirm the new test names it and goes red.',
    },
    newGuardFails: {
      type: 'string',
      description:
        'if this round added a forcing function (a registry that must classify new entries), ' +
        'register a NEW dummy entry and confirm the completeness guard fails naming it.',
    },
    detail: { type: 'string' },
  },
}

// ---- the round ----

phase('Map')
log(`round ${ROUND}: enumerating the surface`)
const inventory = await agent(
  `${CONTEXT}\n\nENUMERATE THE SURFACE.\n${cfg.mapPrompt || 'Produce a complete inventory of the attack surface described above: every entry, whether it is guarded, what it reads/writes/executes/changes, and who can reach it. Work backwards from sinks.'}`,
  { label: `map:r${ROUND}`, phase: 'Map', schema: { type: 'object', additionalProperties: true } }
)

phase('Hunt')
log(`round ${ROUND}: ${LENSES.length} narrow hunters`)
const hunts = await parallel(
  LENSES.map((l) => () =>
    agent(
      `${CONTEXT}\n\nINVENTORY FROM THE MAP PHASE:\n${JSON.stringify(inventory).slice(0, 12000)}\n\nYOUR SURFACE — ${l.key} (you own THIS lens only; depth beats breadth):\n${l.task}`,
      {
        label: `hunt:${l.key}`,
        phase: 'Hunt',
        schema: HUNT_SCHEMA,
        ...(cfg.isolation ? { isolation: 'worktree' } : {}),
      }
    )
  )
)

const results = hunts.filter(Boolean)
// Dedup by normalized title AND against knownGaps. Title-string dedup is imperfect
// (two phrasings of one gap survive) — an accepted limitation, flagged for the closer.
const knownSet = new Set(KNOWN.map((k) => k.toLowerCase().trim()))
const seen = new Set()
const gaps = []
for (const r of results) {
  for (const g of r.newGaps || []) {
    const k = (g.title || '').toLowerCase().trim()
    if (!k || knownSet.has(k) || seen.has(k)) continue
    seen.add(k)
    gaps.push(g)
  }
}
log(`round ${ROUND}: ${results.length}/${LENSES.length} hunters returned, ${gaps.length} fresh gaps`)

if (!gaps.length) {
  return {
    round: ROUND,
    huntersReturned: results.length,
    huntersTotal: LENSES.length,
    dry: true,
    triedAndClean: results.map((r) => r.triedAndClean),
    closure: null,
    verdict: null,
  }
}

phase('Close')
const closed = await agent(
  `${CONTEXT}

CLOSE THESE ${gaps.length} FINDINGS FROM ROUND ${ROUND}. For each: real hole or false positive?
- Real: add a regression case AND fix EVERY copy that fails it (both twins). Mutation-verify.
- False positive: say so with reasoning and change NOTHING.

Some of these may be two phrasings of one underlying gap (title-dedup is imperfect) — merge them.

${JSON.stringify(gaps, null, 2)}

You own all files. When done, ALL of these must be green — run them and report counts:
${TESTS}

Mutation-verify every fix (break -> named test red -> restore). If you make any existing case
LESS strict, you MUST name it in weakenedAnyCase and prove it programmatically. Delete scratch.`,
  { label: `close:r${ROUND}`, phase: 'Close', schema: CLOSE_SCHEMA }
)

phase('Verify')
const verdict = await agent(
  `${CONTEXT}

You are an INDEPENDENT adversary. The closer below claims these fixes are done. Assume every
new guard is decoration until you prove otherwise. Your job is to catch theatre the closer's
own self-report will not.

CLOSER REPORT:
${JSON.stringify(closed, null, 2)}

Do this:
1. Run every suite yourself and report counts: ${TESTS}
2. For each guard/test the closer added: can you DELETE it and keep every suite green? If yes it
   is decoration — list it in stillDecoration.
3. Re-introduce each ORIGINAL defect and confirm the new test names it and goes red
   (catchesOriginalDefect).
4. If this round added a forcing function, register a NEW dummy entry and confirm the
   completeness guard fails (newGuardFails).
An honest "this fix is still theatre" is worth more here than a clean bill of health.`,
  { label: `verify:r${ROUND}`, phase: 'Verify', schema: VERIFY_SCHEMA }
)

return {
  round: ROUND,
  huntersReturned: results.length,
  huntersTotal: LENSES.length,
  dry: false,
  gaps: gaps.map((g) => ({ severity: g.severity, title: g.title })),
  closure: closed,
  verdict,
}
