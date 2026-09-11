/**
 * Fleet-Manager skills — invocable slash commands installed into the user's
 * personal skills dir for a `manager:true` session, the same way
 * `installSupervisorSkill` installs `/supervise` for the supervisor loop. The
 * DIRECTORY follows the session's harness (`~/.claude/skills` for Claude,
 * `$CODEX_HOME/skills` for Codex — identical SKILL.md format); the doctrine
 * text is deliberately the same on every provider.
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
import * as path from 'path';
import { agentSkillDir } from '../lib/agentSkills';
import type { AgentProvider } from './agentProviders';

const STANDUP_NAME = 'standup';
const CHECKPOINT_NAME = 'checkpoint';

// Superseded skill names — removed on install so a session that got an earlier
// build is not left with orphan skills. 'bearings' and 'stow' are the old
// firstmate vocabulary; 'supervise' is the retired fleet-supervisor role, whose
// SKILL.md is already on every existing user's disk and which nothing else
// would ever delete — left in place it keeps offering a stale /supervise that
// half-works by talking straight to claudemon's REST API on :7891.
const RETIRED_NAMES = ['bearings', 'stow', 'supervise'];

const STANDUP_BODY = `---
name: standup
description: Fleet status at a glance for the Workspacer Fleet Manager — a tight four-section digest of what your dispatched workers are doing. Only useful inside a Workspacer Fleet Manager session (requires the mcp__workspacer__* tools).
---

# /standup — fleet status

Give one concise digest, then stop. No spawning, polling or brief edits.
Use the progress/results already in this conversation and your fleet brief.
Call \`list_agents\` once for live state; read only relevant project briefs
whose recent outcomes or priorities are missing. Use \`project_status\` for
requested git state, with \`dirs\` to scope it. Fetch \`get_conversation\` with
\`lastMessage:true\` only for a missing final report, or \`textOnly:true\` and
\`sinceSeq\` for a specific dialogue gap. Do not repeat a worker's own report
request or generate a model summary when its progress/result already answers.

Use these sections, omitting empty ones:
- **In flight**: session:<id>, project, task, state and last meaningful progress.
  For NEEDS A DECISION, say what answer you will provide or need from the user.
- **Landed recently**: a few newest project outcomes; flag independent review
  still owed under the task's selected policy (legacy ship tasks require it).
- **Waiting on you**: unresolved user decisions and approvals.
- **Next up**: brief-based recommendations only; do not dispatch them here.

Use exact session:<id> references for clickable agent links.
`;

const CHECKPOINT_BODY = `---
name: checkpoint
description: Capture durable knowledge from this Fleet Manager session and file each finding to the most specific home (project brief, fleet brief, or a note for the user), then prune stale brief lines. Only useful inside a Workspacer Fleet Manager session (requires the mcp__workspacer__* tools).
---

# /checkpoint — durable memory and brief maintenance

File knowledge from this conversation that would otherwise be lost. Do not
spawn workers or invent facts. Read only briefs you need to change; preserve
user-authored lines. Add via atomic \`brief_append\`, never a whole-file rewrite;
inspect before editing/removing existing lines.

Every brief is \`<project>/.workspacer/brief.md\`:
- **Now**: live work/escalations; remove landed or abandoned items.
- **Direction**: durable goals, priorities and sequencing.
- **Recently**: newest-first dated outcomes, \`- YYYY-MM-DD  <what happened>\`.
- **User** (fleet only): the user's standing preferences.

Route to the most specific home:
1. Repo knowledge belongs in that project's brief or its existing context
   system. For Rivet findings, use \`rivet.learn\` when available or leave the
   owning worker a concise followup to record them. Without repo memory tools,
   flag permanent CLAUDE.md/AGENTS.md additions for the user.
2. Fleet-wide priorities, dispatch outcomes and escalations belong in your own
   fleet brief. Do not copy project briefs into it.
3. Stated user preferences go in the fleet brief's User section.
4. A task-specific next action goes to its worker via \`send_message\` when
   authorized, or into that project's Now section.

Prune only touched briefs. \`brief_check({project})\` identifies potentially
stale Now references; decide which are actually finished. Use
\`brief_archive({project,section:"Recently",keep:20})\` for old outcomes,
adjusting the count for activity. It preserves entries byte-for-byte in
\`brief.archive.md\` under the brief lock. Do not delete history or hand-write
archive headings. Archive other sections only when the oldest entries are
finished/superseded; never archive active goals merely to meet a count.

\`brief_append\` refuses lines over 4000 characters; split them. Its
entriesInSection/bytesInSection report size without another read. For worker
outcomes pass the validated result and sessionId; write only the significance
in line. Preserve evidence caveats. Finish with a short report of what was
filed, archived and left for the user.
`;

const HANDOFF_NAME = 'handoff';

const HANDOFF_BODY = `---
name: handoff
description: End this Fleet Manager session and write everything a FRESH manager needs to pick the fleet up mid-flight — live dispatches and what each was told, escalations waiting on the user, the immediate next action, and context that only exists in this conversation. Run it when context is nearly spent. Only useful inside a Workspacer Fleet Manager session (requires the mcp__workspacer__* tools).
---

# /handoff — end this session so the next one loses nothing

When the host sends a HOST-OWNED MANAGER HANDOFF request with an operation ID
and an exact JSON output path, follow that request's checkpoint and receipt
protocol. Do not execute the standalone termination/reopen/adoption steps below.
The host creates and binds the successor and transfers workers and tasks.
Do not fall back to a mechanical digest or the shared handoff.md.
The remaining instructions are the standalone /handoff fallback only.

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

## Adopt the fleet — your FIRST action, before anything else
Every fleet wake is routed to a worker's PARENT SESSION: its finished report and
its \`report_progress\` notes go to whatever dispatched it, which is the session
that wrote this file and is gone. Left alone, every worker under "In flight"
finishes SILENTLY. One call moves them onto you:

\`adopt_workers({fromSessionId: "<the id on the 'Written by' line at the top of
this file>", toSessionId: "<your own session id>"})\`

It moves every dispatch still parented to me — including one spawned so recently
it has not reported yet, and a finish already on its way — and from then on they
wake YOU when they land. No polling, no reconciliation by hand. It tells you what
it moved; "nothing was still parented" is a real answer (they all finished
already), not a failure, and it is refused out loud if you are not a live manager
session rather than quietly silencing anyone.

Then, ONCE, \`list_agents\` and reconcile it against the ids above for the ones
that finished BEFORE you adopted them: \`get_conversation\` each, do what its
entry says is owed, and file the outcome. That pass is bounded — only these ids,
only once — and everything after it arrives as a wake, the way it does for a
dispatch you made yourself.

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

## Step 5 — leave the fleet a paper trail as well

Your successor adopts your workers on its first turn, so their reports are no
longer lost by default — but a wake is one message in one conversation, and a
dated brief line outlives it. For each in-flight worker, \`send_message\` it one
short instruction: a ship worker leaves a dated line in its project's brief
"## Recently" when it finishes; a scout writes its findings to the report file
it was told to produce. A queued message does not disturb a worker mid-turn. And
remind it that \`report_progress\` reaches its manager from ANY tier, view scouts
included — one line after the handover tells your successor what it is holding
without waiting for the whole task to land.

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
   re-minted from current config — and reads the handoff on its first turn,
   whose first instruction is to adopt your workers so their wakes follow it.

Then stop. Do not dispatch anything new: a dispatch you make now is a dispatch
nobody is waiting for.
`;

/** Where this skill installs for `provider` — null when that harness has no
 *  personal-skills convention we can write to (see lib/agentSkills). */
function skillDir(provider: AgentProvider, name: string): string | null {
  return agentSkillDir(provider, name);
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
 *
 * `provider` picks the DESTINATION only (`~/.claude/skills` vs
 * `$CODEX_HOME/skills`); the doctrine text is identical on every harness by
 * design — see lib/agentSkills. A provider with no known skills directory is
 * logged and skipped: a manager there still has its doctrine (it rides the
 * kickoff message) and its facade tools, just no slash commands.
 */
export function installManagerSkills(provider: AgentProvider = 'claude', strict = false): void {
  if (!agentSkillDir(provider, STANDUP_NAME)) {
    if (strict) throw new Error('Manager skills cannot be installed for this provider');
    console.warn(
      `[managerSkills] ${provider} has no known personal-skills directory — ` +
        'skipping /standup, /checkpoint and /handoff for this manager',
    );
    return;
  }
  try {
    for (const [name, body] of [
      [STANDUP_NAME, STANDUP_BODY],
      [CHECKPOINT_NAME, CHECKPOINT_BODY],
      [HANDOFF_NAME, HANDOFF_BODY],
    ] as const) {
      const dir = skillDir(provider, name);
      if (!dir) continue;
      fs.mkdirSync(dir, { recursive: true });
      writeIfChanged(path.join(dir, 'SKILL.md'), body);
    }
    // Sweep the old names so a manager from an earlier build isn't left with
    // duplicate /bearings + /stow skills alongside the renamed pair, nor with
    // the retired /supervise.
    for (const old of RETIRED_NAMES) {
      const dir = skillDir(provider, old);
      if (dir) fs.rmSync(dir, { recursive: true, force: true });
    }
  } catch (error) {
    if (strict) throw error;
    /* installing the skills is best-effort for ordinary manager creation */
  }
}
