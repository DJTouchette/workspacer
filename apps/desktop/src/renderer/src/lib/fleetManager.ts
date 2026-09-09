import { WORKFLOW_DISCOVERY } from '../../../main/shared/fleetWorkflow';
/**
 * The Fleet Manager — ONE conversation orchestrating many agents
 * (FLEET_MANAGER_SPIKE.md). A real agent — on whichever harness config
 * agents.managerProvider names — rooted at the user's projects
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
 *   5. Review is a dispatch of its own, to a worker that is not the one that
 *      implemented. The routing spec's Invariant 3: review has to be
 *      independent of implementation, so the reviewer gets the diff and the
 *      criteria and never the implementer's reasoning.
 */
export { buildManagerKickoff } from '../../../main/shared/managerDoctrine';

/**
 * Expand a leading '~' against the home directory.
 *
 * The spawn boundary deliberately does NOT do this — main/lib/spawnCwd.ts
 * trims a caller's cwd and nothing else (BINDING DECISION 1), so '~' reaches
 * the daemon as an ordinary filename and the agent is launched in a directory
 * that does not exist. That rule is about paths arriving over the bus; this
 * value is different in kind: a person typed it into Settings → Projects root,
 * where '~/' is simply how a person spells their home directory. Expanding it
 * HERE, at the one place the setting is read, keeps the boundary's rule intact
 * while the setting means what the person meant.
 *
 * '~user' is left alone: resolving another account's home is not something the
 * renderer can do, and guessing would be worse than passing it through.
 */
function expandHome(p: string, home: string): string {
  if (p !== '~' && !p.startsWith('~/') && !p.startsWith('~\\')) return p;
  return `${home.replace(/[/\\]+$/, '')}${p.slice(1)}`;
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
  const explicit = expandHome((fleetRoot ?? '').trim(), home);
  if (explicit) return explicit;
  const dirs = projectCwds.map((c) => expandHome(c, home).replace(/\/+$/, '')).filter(Boolean);
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

export function buildManagerWorkflowAsk(ask: string): string {
  return `${ask.trim()}\n\n${WORKFLOW_DISCOVERY}`;
}
