/**
 * Where a coding agent looks for the user's personal SKILL.md packages.
 *
 * The Fleet Manager's `/standup`, `/checkpoint` and `/handoff` — and the
 * supervisor's `/supervise` — were installed only into `~/.claude/skills`, so a
 * manager running on any other harness came up without its invocable skills.
 * That is a DESTINATION problem, not a doctrine problem: Codex reads
 * `$CODEX_HOME/skills` (falling back to `~/.codex/skills`) and parses the exact
 * same `SKILL.md` frontmatter — name + description, body loaded when the skill
 * applies. So the same text, written to the right directory, works on both.
 *
 * Deliberately NOT a per-provider fork of the skill CONTENT: the manager
 * doctrine is load-bearing and a second copy would drift. Where a skill must
 * name its own install path (the /supervise helper script), that ONE line is
 * parameterized on the directory resolved here.
 *
 * Providers with no known personal-skills convention return null — their
 * caller logs and skips rather than writing files into a directory the agent
 * will never read.
 */
import * as os from 'os';
import * as path from 'path';
import type { AgentProvider } from '../services/agentProviders';

/**
 * The user's personal skills directory for `provider`, or null when the
 * provider has no skills convention we have verified.
 *
 *  - claude → `~/.claude/skills`
 *  - codex  → `$CODEX_HOME/skills`, else `~/.codex/skills` (the Codex CLI's own
 *    documented location; `~/.codex/skills/.system` holds its built-ins).
 *  - copilot → `~/.copilot/skills` (GitHub Copilot CLI, verified against
 *    v1.0.81: `copilot skill --help` names "Personal ~/.copilot/skills/ or
 *    ~/.agents/skills/", `copilot skill add` "materialize[s] into your personal
 *    skills directory (~/.copilot/skills/<name>/SKILL.md)", and `copilot skill
 *    list` was observed picking up a SKILL.md from the sibling ~/.agents/skills.
 *    Same frontmatter, so the same text works unchanged).
 *
 * OpenCode and Pi are omitted on purpose: neither has a personal-skills path
 * verified against its CLI, and guessing one would silently write files nothing
 * reads — the exact failure shape this module exists to end.
 */
export function agentSkillsRoot(provider: AgentProvider): string | null {
  switch (provider) {
    case 'claude':
      return path.join(os.homedir(), '.claude', 'skills');
    case 'codex': {
      const home = process.env.CODEX_HOME?.trim();
      return path.join(home || path.join(os.homedir(), '.codex'), 'skills');
    }
    case 'copilot':
      return path.join(os.homedir(), '.copilot', 'skills');
    default:
      return null;
  }
}

/** The directory one named skill installs into, or null (see agentSkillsRoot). */
export function agentSkillDir(provider: AgentProvider, name: string): string | null {
  const root = agentSkillsRoot(provider);
  return root ? path.join(root, name) : null;
}
