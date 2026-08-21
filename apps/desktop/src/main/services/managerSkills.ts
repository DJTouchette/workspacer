/**
 * Fleet-Manager skills — invocable slash commands installed into the user's
 * personal Claude Code skills dir for a `manager:true` session, the same way
 * `installSupervisorSkill` installs `/supervise` for the supervisor loop.
 *
 * Two skills, both leaning on the manager's operator-tier workspacer facade:
 *   /standup    — an on-demand fleet status digest (what's in flight, what
 *                 landed, what's waiting on you, what I'd dispatch next).
 *   /checkpoint — a deliberate "capture durable knowledge and file it to the
 *                 right home" sweep. The manager's brief-handling is otherwise
 *                 reactive (one line on a finish wake); /checkpoint is the
 *                 considered end-of-session pass, with tiered routing and decay.
 *
 * Best-effort: a write failure just means the manager falls back to its
 * doctrine (the kickoff already describes briefs and status reporting).
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const STANDUP_NAME = 'standup';
const CHECKPOINT_NAME = 'checkpoint';

// Superseded firstmate-vocabulary names — removed on install so a session that
// got the earlier build is not left with orphan /bearings and /stow skills.
const RETIRED_NAMES = ['bearings', 'stow'];

const STANDUP_BODY = `---
name: standup
description: Fleet status at a glance for the Workspacer Fleet Manager — a tight four-section digest of what your dispatched workers are doing. Only useful inside a Workspacer Fleet Manager session (requires the mcp__workspacer__* tools).
---

# /standup — where the fleet stands

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

**Waiting on you** — every decision parked on the user: a worker blocked on an
approval you escalated, a question, a merge awaiting the go-ahead.

**Next up** — what you would dispatch next and why, in one line each, guided by
each brief's "## Direction" and the fleet brief's "## User" preferences.
Suggestions only — do not act on them here.

Reference every agent as \`session:<id>\` so the user can click through. Prefer
bullets over prose. This is a read; it must not change any brief or spawn a
worker.
`;

const CHECKPOINT_BODY = `---
name: checkpoint
description: Capture durable knowledge from this Fleet Manager session and file each finding to the most specific home (project brief, fleet brief, or a note for the user), then prune stale brief lines. Only useful inside a Workspacer Fleet Manager session (requires the mcp__workspacer__* tools).
---

# /checkpoint — file what this session learned, then trim

A deliberate sweep of the CURRENT conversation for knowledge that only exists in
chat and would be lost on a restart. Route each finding to the most specific
home, then trim the briefs you touched. Never invent facts, and never rewrite a
line the user wrote: inspect-then-edit (read the file, merge, write), never
blind-append.

Every brief (project AND fleet) is a \`.workspacer/brief.md\` with the same shape:
- **## Now** — in flight, one line each. A LIVE list, not a log — a line leaves
  the moment its work lands or is abandoned.
- **## Direction** — durable goals, priorities, sequencing.
- **## Recently** — a DATED log, newest first: new entries go at the TOP as
  \`- YYYY-MM-DD  <what happened>\`. This is the only section that grows.
- **## User** (fleet brief only) — standing preferences the user has stated.

Routing, most specific first:

1. **Project-intrinsic knowledge** (how a repo builds/tests, a gotcha, a
   convention) → that project's brief, "## Direction" if durable or "## Now" if
   in flight. If it is truly permanent repo memory, tell the user it belongs in
   that project's CLAUDE.md rather than writing it there yourself.
2. **Cross-project / fleet state** (a dispatch outcome, a shifted priority, an
   open escalation) → YOUR fleet brief (\`.workspacer/brief.md\` under your cwd):
   outcomes to "## Recently", priorities/sequencing to "## Direction", open
   dispatches and user-waiting items to "## Now".
3. **A user preference** stated this session → the fleet brief's "## User"
   heading, so it survives restarts — and honor it from then on.
4. **A task-scoped next step** that belongs to one worker → send it to that
   worker with \`send_message\`, or note it in that project's "## Now".

Then PRUNE each brief you touched:
- **## Now** — remove every item whose work has landed or been abandoned.
- **## Recently** — keep only the ~20 newest entries. Do NOT delete the overflow:
  MOVE the oldest entries to \`.workspacer/brief.archive.md\` beside the brief,
  appending them under a \`## <today's date>\` heading (create the file if it does
  not exist). The archive is cold storage — never rewritten, only appended — so
  the brief stays short while the history survives.

Finish with a one-paragraph report: what you filed and where, what you archived,
and what you left for the user to decide. Do not spawn workers; this is
bookkeeping.
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
 * Install the Fleet Manager's `/standup` and `/checkpoint` skills, refreshing
 * them so a manager session always sees the current version, and removing the
 * superseded firstmate-named dirs. Best-effort and safe to call on every
 * manager spawn — the twin of installSupervisorSkill.
 */
export function installManagerSkills(): void {
  try {
    for (const [name, body] of [
      [STANDUP_NAME, STANDUP_BODY],
      [CHECKPOINT_NAME, CHECKPOINT_BODY],
    ] as const) {
      const dir = skillDir(name);
      fs.mkdirSync(dir, { recursive: true });
      writeIfChanged(path.join(dir, 'SKILL.md'), body);
    }
    // Sweep the old names so a manager from the earlier build isn't left with
    // duplicate /bearings + /stow skills alongside the renamed pair.
    for (const old of RETIRED_NAMES) {
      fs.rmSync(skillDir(old), { recursive: true, force: true });
    }
  } catch {
    /* installing the skills is best-effort */
  }
}
