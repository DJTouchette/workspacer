/**
 * App-owned agent skills, delivered per session as harness plugins.
 *
 * The skills (spawn-agent, project-brief, scheduled-jobs and response cards for
 * ordinary agents; standup, checkpoint, handoff and response cards for Fleet
 * Managers) are bundled at build time from `assets/skills` and written once to
 * a content-addressed directory under `~/.workspacer/agent-skills/<version>/`,
 * laid out as one Claude Code plugin per role:
 *
 *   <version>/workspacer/.claude-plugin/plugin.json
 *   <version>/workspacer/skills/<skill>/SKILL.md
 *   <version>/workspacer-fleet/...
 *
 * Each launch then points its harness at the role's plugin for THAT session only:
 *
 *  - Claude (PTY and stream): `--plugin-dir <plugin>` — real skills, namespaced
 *    `workspacer:<skill>`, listed in the init frame.
 *  - Codex on an app-server (headless and hybrid): claudemon's `skill_roots`,
 *    which it applies with `skills/extraRoots/set` on the session's own server.
 *  - Everything else (Copilot, OpenCode, Codex's PTY-only rollout path): a
 *    pointer line naming each SKILL.md.
 *
 * Nothing is written into the project or a harness's persistent discovery
 * roots, so a Fleet Manager never discovers ordinary skills (or the reverse)
 * from an earlier launch. The `instruction` line also covers a plugin that did
 * not load (managed policy, `--safe-mode`, an older CLI): it names the files.
 *
 * Twin: services/hub-rs/src/services/launch_instructions.rs (native/headless).
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import type { AgentProvider } from './agentProviders';
import bundle from './agentSkillPlugins.generated.json';

export type AgentSkillRole = 'ordinary' | 'manager';
/** How the launching harness takes the role's skills; see the module header. */
export type AgentSkillLoading = 'plugin-dir' | 'skill-roots' | 'pointer';

export const AGENT_SKILL_PLUGINS: Record<AgentSkillRole, string> = {
  ordinary: 'workspacer',
  manager: 'workspacer-fleet',
};
export const AGENT_SKILLS_VERSION: string = bundle.version;
const FILES: Record<string, string> = bundle.files;
const INSTRUCTIONS: Record<string, { native: string; pointer: string }> = bundle.instructions;
const PLUGIN_SKILLS: Record<string, string[]> = bundle.plugins;

export interface AgentSkillLaunch {
  /** Claude argv: `--plugin-dir <plugin>`; empty for other loadings. */
  args: string[];
  /** Codex app-server skill roots (claudemon `skill_roots`); empty otherwise. */
  skillRoots: string[];
  /** One instruction line naming the skills and their files; '' when none. */
  instruction: string;
}

const NONE: AgentSkillLaunch = { args: [], skillRoots: [], instruction: '' };

/** The default way `provider` takes skills. Codex's PTY-only rollout launch
 *  has no app-server to configure and passes 'pointer' explicitly. */
export function agentSkillLoading(provider: AgentProvider): AgentSkillLoading {
  if (provider === 'claude') return 'plugin-dir';
  if (provider === 'codex') return 'skill-roots';
  return 'pointer';
}

export function agentSkillBundleRoot(home: string = os.homedir()): string {
  return path.join(home, '.workspacer', 'agent-skills', AGENT_SKILLS_VERSION);
}

/** Create `dir` (and its parents), refusing a symlink or non-directory. */
function ownedDir(dir: string): void {
  fs.mkdirSync(dir, { recursive: true });
  const stat = fs.lstatSync(dir);
  if (stat.isSymbolicLink() || !stat.isDirectory()) {
    throw new Error(`agent skills: ${dir} is not an owned directory`);
  }
}

/** Write every bundled file under the versioned root, verifying what is
 *  already there. The directory is app-owned and content-addressed, so a
 *  missing or altered file is restored (atomically, by rename) rather than
 *  trusted; a symlink anywhere in the tree refuses the whole root. Returns the
 *  root, or null when it cannot be made trustworthy. */
export function materializeAgentSkills(home: string = os.homedir()): string | null {
  const root = agentSkillBundleRoot(home);
  try {
    for (const dir of [
      path.join(home, '.workspacer'),
      path.join(home, '.workspacer', 'agent-skills'),
      root,
    ]) {
      ownedDir(dir);
    }
    for (const [rel, body] of Object.entries(FILES)) {
      const file = path.join(root, ...rel.split('/'));
      let dir = root;
      for (const part of path.relative(root, path.dirname(file)).split(path.sep)) {
        if (!part) continue;
        dir = path.join(dir, part);
        ownedDir(dir);
      }
      try {
        const stat = fs.lstatSync(file);
        if (stat.isSymbolicLink()) throw new Error(`agent skills: ${file} is a symlink`);
        if (stat.isFile() && fs.readFileSync(file, 'utf8') === body) continue;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      }
      const tmp = `${file}.${process.pid}.${Math.random().toString(36).slice(2)}.tmp`;
      fs.writeFileSync(tmp, body, { encoding: 'utf8', flag: 'wx' });
      fs.renameSync(tmp, file);
    }
    return root;
  } catch (error) {
    console.warn('[agentSkills] could not prepare agent skills:', error);
    return null;
  }
}

/** The instruction line for `plugin`, with `{dir}` / `{skill:<name>}` bound to
 *  JSON-quoted absolute paths (same substitution as the Rust twin). */
export function agentSkillInstruction(
  pluginDir: string,
  plugin: string,
  mode: 'native' | 'pointer',
): string {
  const skills = path.join(pluginDir, 'skills');
  return INSTRUCTIONS[plugin][mode].replace(/\{(dir|skill:[a-z0-9-]+)\}/g, (_, key: string) =>
    JSON.stringify(
      key === 'dir' ? skills : path.join(skills, key.slice('skill:'.length), 'SKILL.md'),
    ),
  );
}

/**
 * Prepare one launch's skills: clear what older builds left in discovery roots,
 * materialize the bundle, and return the harness-specific pieces. Pi gets
 * nothing (no MCP bridge, so tool-driven skills would be decorative). `strict`
 * (manager replacement) throws instead of degrading to no skills.
 */
export function prepareAgentSkills(
  provider: AgentProvider,
  cwd: string,
  opts: {
    manager?: boolean;
    loading?: AgentSkillLoading;
    strict?: boolean;
    home?: string;
  } = {},
): AgentSkillLaunch {
  const home = opts.home ?? os.homedir();
  removeLegacyAgentSkills(cwd, home);
  if (provider === 'pi') return NONE;
  const root = materializeAgentSkills(home);
  if (!root) {
    if (opts.strict) throw new Error('Workspacer agent skills could not be prepared');
    return NONE;
  }
  const plugin = AGENT_SKILL_PLUGINS[opts.manager ? 'manager' : 'ordinary'];
  const pluginDir = path.join(root, plugin);
  const loading = opts.loading ?? agentSkillLoading(provider);
  return {
    args: loading === 'plugin-dir' ? ['--plugin-dir', pluginDir] : [],
    skillRoots: loading === 'skill-roots' ? [path.join(pluginDir, 'skills')] : [],
    instruction: agentSkillInstruction(
      pluginDir,
      plugin,
      loading === 'pointer' ? 'pointer' : 'native',
    ),
  };
}

// ── Migration cleanup ───────────────────────────────────────────────────────
//
// Older builds wrote these skills into discovery roots: the project's
// `.claude/skills` / `.agents/skills`, a pointer copy under the project's
// `.workspacer/skills/<hash>/`, and (Fleet Manager) the user's personal
// `~/.claude/skills` / `$CODEX_HOME/skills`. Project copies are removed only
// when byte-identical to a bundled file; a symlinked parent is never followed.

/** skill-relative path ("spawn-agent/SKILL.md") → every bundled body for it. */
const SKILL_FILES: Map<string, Set<string>> = (() => {
  const map = new Map<string, Set<string>>();
  for (const [rel, body] of Object.entries(FILES)) {
    const match = /^[^/]+\/skills\/(.+)$/.exec(rel);
    if (!match) continue;
    if (!map.has(match[1])) map.set(match[1], new Set());
    map.get(match[1])!.add(body);
  }
  return map;
})();

const MANAGER_SKILLS = PLUGIN_SKILLS[AGENT_SKILL_PLUGINS.manager].filter((name) =>
  ['standup', 'checkpoint', 'handoff'].includes(name),
);

function realDir(dir: string): boolean {
  try {
    const stat = fs.lstatSync(dir);
    return stat.isDirectory() && !stat.isSymbolicLink();
  } catch {
    return false;
  }
}

function rmdirUpTo(dir: string, stop: string): void {
  for (let current = dir; current.startsWith(stop); current = path.dirname(current)) {
    try {
      fs.rmdirSync(current);
    } catch {
      return;
    }
    if (current === stop) return;
  }
}

/** Remove exact bundled copies under `skillsRoot/<skill>/…` (one level of
 *  `<hash>` first when `hashed`). */
function removeExactCopies(skillsRoot: string, hashed: boolean): void {
  if (!realDir(skillsRoot)) return;
  const bases = hashed
    ? fs
        .readdirSync(skillsRoot)
        .map((name) => path.join(skillsRoot, name))
        .filter(realDir)
    : [skillsRoot];
  for (const base of bases) {
    for (const [rel, bodies] of SKILL_FILES) {
      const parts = rel.split('/');
      let dir = base;
      if (!parts.slice(0, -1).every((part) => realDir((dir = path.join(dir, part))))) continue;
      const file = path.join(base, ...parts);
      try {
        const stat = fs.lstatSync(file);
        if (!stat.isFile() || stat.isSymbolicLink()) continue;
        if (!bodies.has(fs.readFileSync(file, 'utf8'))) continue;
        fs.unlinkSync(file);
        rmdirUpTo(path.dirname(file), skillsRoot);
      } catch {
        // Missing, unreadable or user-controlled entries are left untouched.
      }
    }
  }
}

/** Personal manager copies were Workspacer-owned outright (rewritten on every
 *  manager spawn), but older builds wrote older text, so they are recognized by
 *  their frontmatter rather than by exact bytes. */
function removePersonalManagerSkills(skillsRoot: string): void {
  if (!realDir(skillsRoot)) return;
  for (const name of MANAGER_SKILLS) {
    const dir = path.join(skillsRoot, name);
    const file = path.join(dir, 'SKILL.md');
    try {
      if (!realDir(dir) || fs.readdirSync(dir).join() !== 'SKILL.md') continue;
      const stat = fs.lstatSync(file);
      if (!stat.isFile() || stat.isSymbolicLink()) continue;
      const head = fs.readFileSync(file, 'utf8').slice(0, 1024);
      if (!head.startsWith(`---\nname: ${name}\n`) || !head.includes('Workspacer Fleet Manager'))
        continue;
      fs.unlinkSync(file);
      fs.rmdirSync(dir);
    } catch {
      // Leave anything we cannot positively identify.
    }
  }
}

let personalCleaned = false;

export function removeLegacyAgentSkills(cwd: string, home: string = os.homedir()): void {
  try {
    const project = fs.realpathSync(cwd);
    if (project !== fs.realpathSync(home) && project !== path.parse(project).root) {
      for (const native of ['.claude', '.agents']) {
        if (realDir(path.join(project, native))) {
          removeExactCopies(path.join(project, native, 'skills'), false);
        }
      }
      if (realDir(path.join(project, '.workspacer'))) {
        removeExactCopies(path.join(project, '.workspacer', 'skills'), true);
      }
    }
  } catch {
    // An unusable cwd is the launcher's error to report, not cleanup's.
  }
  if (personalCleaned && home === os.homedir()) return;
  if (home === os.homedir()) personalCleaned = true;
  // CODEX_HOME belongs to the real home; a caller-supplied home (tests) keeps
  // the cleanup inside it.
  const codexHome =
    (home === os.homedir() && process.env.CODEX_HOME?.trim()) || path.join(home, '.codex');
  for (const root of [path.join(home, '.claude', 'skills'), path.join(codexHome, 'skills')]) {
    removePersonalManagerSkills(root);
  }
}
