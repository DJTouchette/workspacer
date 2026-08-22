/**
 * Fleet-Manager skills — invocable slash commands installed into the user's
 * personal Claude Code skills dir for a `manager:true` session, the same way
 * `installSupervisorSkill` installs `/supervise` for the supervisor loop.
 *
 * Three skills, all leaning on the manager's operator-tier workspacer facade:
 *   /standup    — an on-demand fleet status digest (what's in flight, what
 *                 landed, what's waiting on you, what I'd dispatch next).
 *   /checkpoint — a deliberate "capture durable knowledge and file it to the
 *                 right home" sweep. The manager's brief-handling is otherwise
 *                 reactive (one line on a finish wake); /checkpoint is the
 *                 considered end-of-session pass, with tiered routing and decay.
 *   /handoff    — end-of-context succession: run /checkpoint for the durable
 *                 half, then write <fleet root>/.workspacer/handoff.md with the
 *                 MID-FLIGHT half a successor cannot re-derive (live dispatches
 *                 and what each was told, open escalations, the next action,
 *                 conversation-only context). Distinct from claudemon's
 *                 per-session ~/.workspacer/handoffs/ briefs, which distill ONE
 *                 agent's transcript for a cross-provider successor; this is the
 *                 manager's whole fleet posture, authored not derived.
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
   in flight. If the project uses rivet, a durable finding's proper home is
   \`rivet.learn\` (its context docs) — have the worker record it there. If it is
   truly permanent repo memory with no rivet, tell the user it belongs in that
   project's CLAUDE.md rather than writing it there yourself.
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

const HANDOFF_NAME = 'handoff';

const HANDOFF_BODY = `---
name: handoff
description: End this Fleet Manager session and write everything a FRESH manager needs to pick the fleet up mid-flight — live dispatches and what each was told, escalations waiting on the user, the immediate next action, and context that only exists in this conversation. Run it when context is nearly spent. Only useful inside a Workspacer Fleet Manager session (requires the mcp__workspacer__* tools).
---

# /handoff — end this session so the next one loses nothing

Your context is nearly spent and the user wants to continue in a fresh session.
You are writing to a SUCCESSOR: a Fleet Manager that boots with your briefs,
your fleet, your projects — and none of your conversation. Everything it cannot
re-derive from disk dies with you unless you write it down now.

## The line between /checkpoint and /handoff — do not blur it

**/checkpoint files what should OUTLIVE the session. /handoff records what a
successor needs to RESUME MID-FLIGHT.** Checkpoint's output is still true next
week; handoff's output is true for the next hour, and the successor deletes it
the moment it has absorbed it.

So: **run /checkpoint first, in full, as your literal first step — invoke the
skill, do not reimplement it.** It is the one place that knows the brief shape,
the routing order, and the archival rules; a second copy of that logic here
would drift from it within a release. When it returns, everything durable is
already filed where it belongs, and what is left in your head is exactly this
skill's subject matter. Use that as your test for every line you are about to
write: if it would still matter next week, it is checkpoint's, not yours — put
it in a brief and let the handoff point at the brief.

**Pointers, never copies.** Never restate a brief in the handoff. Link it
(\`<abs path>/.workspacer/brief.md\`) and say in one clause why the successor
should open it. A handoff that duplicates its own sources goes stale against
them the first time anyone edits either one.

## Step 1 — /checkpoint

Run it. Wait for it. Do not continue until it reports.

## Step 2 — inventory, from cheapest source to dearest

- \`list_agents\` — every live session, its id, label, cwd, and state. This is
  the authoritative list of what is in flight; your memory of it is not.
- \`get_config\` — \`projects[<dir>].delivery\` for each project holding a live
  dispatch, so the successor lands the work the way that project wants.
- **Your own conversation** — the only source for what each worker was actually
  TOLD, what you owe it when it lands, what the user asked that you never
  answered, and what you were about to do next. Re-read it deliberately; this
  is the part that is about to be deleted.

## Step 3 — write \`<your cwd>/.workspacer/handoff.md\`

One file, beside your fleet brief. Overwrite any existing one — a handoff is
never a log. Use \`write_file\`. Follow this shape; drop a section only if it is
genuinely empty, and keep every session id exact.

\`\`\`markdown
# Fleet handoff — <YYYY-MM-DD HH:MM>
Written by session:<your own session id> at <your cwd>.
You are the successor. Read this once, act on it, then follow "Close out" and
delete this file. If every session id below is dead and the date is stale, this
handoff has already been consumed — delete it and carry on from the briefs.

## Read first
- Fleet brief: ./brief.md — durable fleet state, checkpointed just before this.
- <project>: <abs path>/.workspacer/brief.md — <one clause: why it matters now>

## In flight (<n>)
### session:<id> — <project> — <the task in one line>
- Told to: <the substance of the dispatch in 1-3 lines — the task, the
  constraints it was given, and how it was told to report back>
- Shape: ship|scout · worktree <yes: abs path | no> · delivery <local|pr> ·
  tier <view|triage|operator> · provider <claude|codex|…> · dispatched <time>
- When it lands I owe it: <the exact follow-up — merge the branch into X, prepend
  a dated line to Y's brief, tell the user Z, dispatch the follow-up W>
- User has been told: <what the user already knows about this one, or "nothing">

## Waiting on the user (<n>)
- <the question or decision, worded as it was put to them> — asked <when>, via
  <notify|chat> — blocks: session:<id> | nothing — my recommendation: <one line>

## Established in conversation only
- <a preference, a correction the user made, a decision half-taken — stated as a
  fact the successor can act on, with enough of the why that it can apply it to
  a case I never saw>

## Next action
- <the ONE thing I was about to do, concrete enough that the successor can just
  do it without reconstructing my reasoning>

## Orphaned wakes
The worker-finished wake is routed to a worker's PARENT SESSION — the session
that wrote this file, which is gone. Every worker under "In flight" will finish
SILENTLY: no [fleet] message will ever reach you about it. So, ONCE, on your
first turn, \`list_agents\` and reconcile it against the ids above; for any that
has already finished, \`get_conversation\` it, do what that entry says is owed,
and file the outcome. For any still running, re-check those specific ids each
time the user next speaks to you, until each one is resolved. This is the one
sanctioned exception to NEVER POLL, and it is bounded: only these ids, only
when the user is already talking to you, and it ends when the list empties.

## Close out
When every "In flight" entry is resolved and every fact above is either acted on
or filed into a brief: delete this file and remove the HANDOFF PENDING line from
the fleet brief's "## Now".
\`\`\`

## Step 4 — make it impossible to miss

Prepend one line to the fleet brief's "## Now" (inspect-then-edit, never blind-
append):

\`- HANDOFF PENDING → .workspacer/handoff.md (written <date> by session:<your id>) — read it before anything else\`

The successor reads its fleet brief on its first turn no matter what, so the
pointer is what guarantees discovery even on an older build whose doctrine does
not mention handoffs. It removes the line as part of "Close out".

## Step 5 — leave the fleet a paper trail, not a promise

For each in-flight worker, \`send_message\` it one short instruction so its
result survives the lost wake: a ship worker leaves a dated line in its
project's brief "## Recently" when it finishes; a scout writes its findings to
the report file it was told to produce. A queued message does not disturb a
worker mid-turn. Now the successor learns the outcome from disk even if it
never reads the transcript.

## Step 6 — hand it to the user, and be honest that you cannot do it yourself

**You cannot start the successor.** Do not try. \`spawn_agent\` has no manager
role: a session you spawn comes up without the manager doctrine, without the
operator grants, without /standup, /checkpoint and /handoff, and unregistered as
a wake target — a manager-shaped agent that silently is not one. Terminating
yourself is no better: the card would go Stopped, and the next Fleet Manager
click RESUMES your conversation, which is the exact thing the user is trying to
escape.

So finish by telling the user, in this order:

1. The handoff is written — name the file and how many dispatches are in flight.
2. Right-click the **Fleet Manager** card in the sidebar → **Terminate**. That
   kills this session AND drops the card, so nothing resumes it.
3. Open the Fleet Manager again from the Overview. With no card to reuse, it
   spawns a genuinely fresh session — new context, same role, same grants,
   re-minted from current config — and reads the handoff on its first turn.

Then stop. Do not dispatch anything new: a dispatch you make now is a dispatch
nobody is waiting for.
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
 * Install the Fleet Manager's `/standup`, `/checkpoint` and `/handoff` skills, refreshing
 * them so a manager session always sees the current version, and removing the
 * superseded firstmate-named dirs. Best-effort and safe to call on every
 * manager spawn — the twin of installSupervisorSkill.
 */
export function installManagerSkills(): void {
  try {
    for (const [name, body] of [
      [STANDUP_NAME, STANDUP_BODY],
      [CHECKPOINT_NAME, CHECKPOINT_BODY],
      [HANDOFF_NAME, HANDOFF_BODY],
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
