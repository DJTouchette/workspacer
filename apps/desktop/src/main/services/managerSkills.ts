/**
 * Fleet-Manager skills for harnesses WITHOUT per-session skill loading.
 *
 * Claude and Codex managers receive `/standup`, `/checkpoint` and `/handoff` as
 * the `workspacer-fleet` plugin for their session only (see agentSkillPlugins).
 * A harness Workspacer cannot hand skills to per launch — today Copilot — still
 * gets them installed into its personal skills directory so they stay
 * invocable as slash commands. The text comes from the same bundle, never a
 * per-provider copy:
 *   /standup    — an on-demand fleet status digest.
 *   /checkpoint — file this session's durable knowledge to the right home.
 *   /handoff    — end-of-context succession: /checkpoint for the durable half,
 *                 then <fleet root>/.workspacer/handoff.md for the mid-flight
 *                 half a successor cannot re-derive.
 *
 * Best-effort: a write failure just means the manager falls back to its
 * doctrine and the pointer line the launcher adds.
 */
import * as fs from 'fs';
import * as path from 'path';
import { agentSkillDir } from '../lib/agentSkills';
import type { AgentProvider } from './agentProviders';
import { AGENT_SKILL_PLUGINS, agentSkillLoading } from './agentSkillPlugins';
import bundle from './agentSkillPlugins.generated.json';

const MANAGER_SKILLS = ['standup', 'checkpoint', 'handoff'] as const;

// Superseded skill names — removed on install so a session that got an earlier
// build is not left with orphan skills. 'bearings' and 'stow' are the old
// firstmate vocabulary; 'supervise' is the retired fleet-supervisor role, whose
// SKILL.md is already on every existing user's disk and which nothing else
// would ever delete — left in place it keeps offering a stale /supervise that
// half-works by talking straight to claudemon's REST API on :7891.
const RETIRED_NAMES = ['bearings', 'stow', 'supervise'];

/** A manager skill's SKILL.md, from the bundled plugin. */
export function managerSkillBody(name: (typeof MANAGER_SKILLS)[number]): string {
  return (bundle.files as Record<string, string>)[
    `${AGENT_SKILL_PLUGINS.manager}/skills/${name}/SKILL.md`
  ];
}

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
 * Install the Fleet Manager's `/standup`, `/checkpoint` and `/handoff` into the
 * personal skills directory of a harness with no per-session loading,
 * refreshing them and sweeping retired names. Claude and Codex are refused:
 * a persistent copy there would surface in every later session.
 */
export function installManagerSkills(provider: AgentProvider, strict = false): void {
  if (agentSkillLoading(provider) !== 'pointer' || !agentSkillDir(provider, 'standup')) {
    if (strict) throw new Error('Manager skills cannot be installed for this provider');
    console.warn(
      `[managerSkills] ${provider} has no personal-skills install path — ` +
        'skipping /standup, /checkpoint and /handoff for this manager',
    );
    return;
  }
  try {
    for (const name of MANAGER_SKILLS) {
      const dir = skillDir(provider, name);
      if (!dir) continue;
      fs.mkdirSync(dir, { recursive: true });
      writeIfChanged(path.join(dir, 'SKILL.md'), managerSkillBody(name));
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
