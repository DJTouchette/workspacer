/**
 * The Fleet Manager — ONE conversation orchestrating many agents
 * (FLEET_MANAGER_SPIKE.md). A real Claude agent rooted at the user's projects
 * parent directory, holding the workspacer MCP facade at the OPERATOR tier,
 * whose whole job is delegation: inventory the projects, dispatch real agents
 * into them, relay results, and keep every turn short enough that the user
 * can always talk to it.
 *
 * Sibling of lib/guide.ts and follows its proven shape: a fixed agent name
 * (reuse-by-name), a compact role preamble, an auto-SENT kickoff message
 * (never a composer pre-fill — Ask-the-Fleet's mistake), and preset chips.
 */

/** Display name of the manager's workspace. Also how a running manager is
 *  recognized for reuse (passed as the user-set name, which protects it from
 *  auto-titling). */
export const FLEET_MANAGER_NAME = 'Fleet Manager';

/**
 * The role doctrine, prepended to the manager's first message. The order of
 * the rules is the order of their importance:
 *   1. Pure delegator — the manager's availability IS the feature.
 *   2. Dispatch through the facade so workers are real, visible agents.
 *   3. Briefs are how it (and the user) know each project's state — including
 *      its OWN fleet brief at <cwd>/.workspacer/brief.md, which is its memory
 *      across restarts (cross-project state only, never a mirror of the
 *      project briefs).
 */
const MANAGER_PREAMBLE =
  'You are the Fleet Manager for this machine: a delegating chief-of-staff for every ' +
  'project under your working directory. You have workspacer MCP tools ' +
  '(mcp__workspacer__*) at the operator tier — call the "help" tool first to learn them.\n\n' +
  'DOCTRINE, in priority order:\n' +
  '1. You DELEGATE — you never edit code, run builds, or do long investigations yourself. ' +
  'Every turn of yours should end in seconds so the user can always reach you. Dispatch a ' +
  'worker instead: spawn_agent with the project directory as cwd, a short label naming the ' +
  'task, and parentSessionId set to your own session so the worker nests under you in the ' +
  'sidebar. Give the worker a complete first message: the task, the relevant context from ' +
  'the project brief, and how to report back.\n' +
  '2. Do not poll workers. You will be WOKEN with a [fleet] message when a worker finishes ' +
  'or blocks; until then, stay idle. When woken: read the result (get_conversation with ' +
  'sinceSeq for detail), update the project brief, and give the user a one-paragraph ' +
  'report with session:<id> references.\n' +
  '3. Every project keeps a living brief at .workspacer/brief.md inside the repo, with ' +
  'sections "## Now" (in flight), "## Direction" (where it is going), "## Recently" ' +
  '(append-only, newest first — prune past ~20 lines). On your FIRST turn: read YOUR OWN ' +
  'fleet brief at .workspacer/brief.md under your cwd (it is your memory across restarts ' +
  '— trust it before re-deriving anything), then list the project directories under your ' +
  'cwd, read each project brief that exists (plus the projects config via the facade), ' +
  'and create missing briefs with what you can infer. When a worker finishes, append one ' +
  'line to that project’s "## Recently" and adjust "## Now". The user’s own edits to a ' +
  'brief are authoritative — never rewrite their words.\n' +
  '4. Your fleet brief holds ONLY cross-project state — never mirror the project briefs ' +
  'into it: "## Now" = open dispatches and escalations waiting on the user, ' +
  '"## Direction" = priorities and sequencing across projects, "## Recently" = dispatch ' +
  'outcomes. Update it whenever you dispatch, get a [fleet] wake, or escalate.\n' +
  '5. Approvals: you may approve a worker’s permission prompts when the action stays ' +
  'inside the repo you dispatched it to (edits, tests, builds). Escalate to the user ' +
  '(notify) for anything destructive, cross-repo, credential-touching, or surprising.\n' +
  '6. Be concrete and brief. Prefer bullet status over prose. Reference agents as ' +
  'session:<id> so the user can click through.\n\n' +
  'The user says:';

/** Compose the manager's first (auto-sent) message from a user ask. */
export function buildManagerKickoff(ask: string): string {
  return `${MANAGER_PREAMBLE}\n\n${ask.trim()}`;
}

/**
 * Resolve the manager's home directory: the explicit config
 * (agents.fleetRoot) wins; else the COMMON PARENT of the configured projects
 * (the directory that visibly "contains the user's work"); else the home
 * directory. Pure — pass inputs explicitly so tests need no config plumbing.
 */
export function deriveFleetRoot(
  fleetRoot: string | undefined,
  projectCwds: string[],
  home: string,
): string {
  const explicit = (fleetRoot ?? '').trim();
  if (explicit) return explicit;
  const dirs = projectCwds.map((c) => c.replace(/\/+$/, '')).filter(Boolean);
  if (dirs.length > 0) {
    let parts = dirs[0].split('/');
    for (const d of dirs.slice(1)) {
      const dp = d.split('/');
      let i = 0;
      while (i < parts.length && i < dp.length && parts[i] === dp[i]) i++;
      parts = parts.slice(0, i);
    }
    // The projects themselves are one level BELOW the root we want, so a
    // common prefix equal to a project is that project's parent. Require a
    // root deeper than '/home' so one stray path can't send the manager to /.
    const common = parts.join('/');
    const root = dirs.includes(common) ? common.split('/').slice(0, -1).join('/') : common;
    if (root.split('/').filter(Boolean).length >= 2) return root;
  }
  return home;
}

export interface ManagerPreset {
  id: string;
  label: string;
  prompt: string;
}

/** Preset asks — surfaced as chips on the Overview hero row. */
export const MANAGER_PRESETS: ManagerPreset[] = [
  {
    id: 'state',
    label: "What's the state of my projects?",
    prompt:
      'Inventory the projects under your directory: read every brief, then give me a tight per-project status — what is in flight, what landed recently, and what you would dispatch next.',
  },
  {
    id: 'briefs',
    label: 'Set up project briefs',
    prompt:
      'Walk the project directories, create or refresh each .workspacer/brief.md from what you can infer (recent commits, READMEs, the projects config), and show me the Now/Direction sections for review.',
  },
  {
    id: 'status',
    label: 'Status of dispatched work',
    prompt:
      'Report on every worker you have dispatched: which finished (and their outcomes), which are still running, which are blocked on me.',
  },
];
