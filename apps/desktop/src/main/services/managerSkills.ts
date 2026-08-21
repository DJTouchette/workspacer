/**
 * Fleet-Manager skills — invocable slash commands installed into the user's
 * personal Claude Code skills dir for a `manager:true` session, the same way
 * `installSupervisorSkill` installs `/supervise` for the supervisor loop.
 *
 * Two skills, both leaning on the manager's operator-tier workspacer facade:
 *   /bearings — an on-demand fleet status digest (what's in flight, what
 *               landed, what's waiting on you, what I'd dispatch next).
 *   /stow     — a deliberate "capture durable knowledge and file it to the
 *               right home" sweep. The manager's brief-handling is otherwise
 *               reactive (one line on a finish wake); /stow is the considered
 *               end-of-session pass, with tiered routing and decay.
 *
 * Best-effort: a write failure just means the manager falls back to its
 * doctrine (the kickoff already describes briefs and status reporting).
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const BEARINGS_NAME = 'bearings';
const STOW_NAME = 'stow';

const BEARINGS_BODY = `---
name: bearings
description: Fleet status at a glance for the Workspacer Fleet Manager — a tight four-section digest of what your dispatched workers are doing. Only useful inside a Workspacer Fleet Manager session (requires the mcp__workspacer__* tools).
---

# /bearings — where the fleet stands

Produce ONE concise digest and stop. Do not spawn anything; do not poll. Read,
compose, report. Sources, cheapest first: your own fleet brief
(.workspacer/brief.md under your cwd), each project's brief, and \`list_agents\`
for live state. Use \`get_conversation\` with \`sinceSeq\` only if you need a
worker's latest outcome you have not already recorded.

Emit exactly these four sections (drop a section only if it is genuinely empty):

**In flight** — each running dispatch: \`session:<id>\` — project — one-line task,
and whether it is working, waiting on approval, or blocked.

**Landed recently** — the newest entries across the project briefs' "## Recently"
(a few lines, newest first), each with its project.

**Waiting on you** — every decision parked on the captain: a worker blocked on an
approval you escalated, a question, a merge awaiting the go-ahead.

**Next up** — what you would dispatch next and why, in one line each. Suggestions
only — do not act on them here.

Reference every agent as \`session:<id>\` so the captain can click through. Prefer
bullets over prose. This is a read; it must not change any brief or spawn a
worker.
`;

const STOW_BODY = `---
name: stow
description: Capture durable knowledge from this Fleet Manager session and file each finding to the most specific home (project brief, fleet brief, or a note for the captain), then prune stale brief lines. Only useful inside a Workspacer Fleet Manager session (requires the mcp__workspacer__* tools).
---

# /stow — file what this session learned, then trim

A deliberate sweep of the CURRENT conversation for knowledge that only exists in
chat and would be lost on a restart. Route each finding to the most specific
home; never invent facts, and never overwrite the captain's own words.

Routing, most specific first:

1. **Project-intrinsic knowledge** (how a repo builds/tests, a gotcha, a
   convention) → that project's \`.workspacer/brief.md\` "## Direction" (durable)
   or "## Now" (in flight). If it is truly permanent and belongs in the repo's
   own agent memory, tell the captain it is worth adding to that project's
   CLAUDE.md rather than writing it yourself.
2. **Cross-project / fleet state** (a dispatch outcome, a shifted priority, an
   open escalation) → YOUR fleet brief at \`.workspacer/brief.md\` under your cwd:
   outcomes to "## Recently", priorities/sequencing to "## Direction", open
   dispatches and captain-waiting items to "## Now".
3. **A captain preference** stated this session (how they like work delivered,
   a standing instruction) → record it under a "## Captain" heading in your fleet
   brief so it survives restarts, and honor it from then on.
4. **A task-scoped next step** that belongs to one worker → send it to that
   worker with \`send_message\`, or note it in that project's "## Now".

Then DECAY: in each brief you touched, prune "## Recently" to about the last 20
lines (oldest first), and drop "## Now" items that are done. Use
inspect-then-edit (read the current file, merge, write) — do not blind-append,
and never rewrite a line the captain wrote.

Finish with a one-paragraph report: what you filed and where, and what you left
for the captain to decide. Do not spawn workers; this is bookkeeping.
`;

function skillDir(name: string): string {
  return path.join(os.homedir(), '.claude', 'skills', name);
}

/** Write `file` only if changed, to avoid churning the user's files/watchers. */
function writeIfChanged(file: string, content: string): void {
  let current = '';
  try {
    current = fs.readFileSync(file, 'utf8');
  } catch {
    /* not installed yet */
  }
  if (current !== content) fs.writeFileSync(file, content, 'utf8');
}

/**
 * Install the Fleet Manager's `/bearings` and `/stow` skills, refreshing them so
 * a manager session always sees the current version. Best-effort and safe to
 * call on every manager spawn — the twin of installSupervisorSkill.
 */
export function installManagerSkills(): void {
  try {
    for (const [name, body] of [
      [BEARINGS_NAME, BEARINGS_BODY],
      [STOW_NAME, STOW_BODY],
    ] as const) {
      const dir = skillDir(name);
      fs.mkdirSync(dir, { recursive: true });
      writeIfChanged(path.join(dir, 'SKILL.md'), body);
    }
  } catch {
    /* installing the skills is best-effort */
  }
}
