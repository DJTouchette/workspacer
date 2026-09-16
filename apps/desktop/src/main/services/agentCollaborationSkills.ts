/** App-owned ordinary-agent skills. They live only under Workspacer's own
 * versioned project directory and are pointed out to ordinary sessions through
 * their private instruction channel. Never install them in a harness-native
 * discovery root: a later Fleet Manager in the same project would discover
 * that persistent copy before Workspacer had any chance to withhold it.
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { createHash } from 'crypto';
import type { AgentProvider } from './agentProviders';
import files from './agentCollaborationSkills.generated.json';

export const AGENT_COLLABORATION_SKILL_NAMES = ['spawn-agent', 'project-brief'] as const;
const version = createHash('sha256').update(JSON.stringify(files)).digest('hex').slice(0, 16);

export function agentCollaborationSkillsRoot(cwd: string): string {
  return path.join(cwd, '.workspacer', 'skills', version);
}

// Refuse symlinks at every destination component. Exclusive file creation
// never overwrites a user-owned file; compare every existing file first.
function installInto(cwd: string, dir: string): boolean {
  try {
    const root = fs.realpathSync(cwd);
    if (root === fs.realpathSync(os.homedir()) || root === path.parse(root).root) return false;
    const relative = path.relative(cwd, dir);
    if (relative.startsWith('..') || path.isAbsolute(relative)) return false;
    let current = root;
    for (const part of relative.split(path.sep)) {
      current = path.join(current, part);
      try {
        fs.mkdirSync(current);
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'EEXIST') return false;
      }
      if (!fs.lstatSync(current).isDirectory() || fs.lstatSync(current).isSymbolicLink())
        return false;
    }
    for (const [rel, body] of Object.entries(files)) {
      const file = path.join(dir, rel);
      const parent = path.dirname(file);
      if (
        parent !== dir &&
        fs.existsSync(parent) &&
        (!fs.lstatSync(parent).isDirectory() || fs.lstatSync(parent).isSymbolicLink())
      )
        return false;
      try {
        if (
          !fs.lstatSync(file).isFile() ||
          fs.lstatSync(file).isSymbolicLink() ||
          fs.readFileSync(file, 'utf8') !== body
        )
          return false;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ENOENT') return false;
      }
    }
    for (const [rel, body] of Object.entries(files)) {
      const file = path.join(dir, rel);
      fs.mkdirSync(path.dirname(file), { recursive: true });
      try {
        fs.writeFileSync(file, body, { encoding: 'utf8', flag: 'wx' });
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'EEXIST') return false;
      }
    }
    return true;
  } catch {
    return false;
  }
}

/** Remove only native copies written by older Workspacer builds. Exact body
 * matching is deliberate: a user-owned collision is never deleted. This is a
 * migration defence for projects which already received the old persistent
 * `.claude/skills` / `.agents/skills` files before manager exclusion became a
 * hard invariant. */
function removeLegacyNativeCopies(provider: AgentProvider, cwd: string): void {
  const native = provider === 'claude' ? '.claude' : provider === 'codex' ? '.agents' : null;
  if (!native) return;
  for (const [rel, body] of Object.entries(files)) {
    const file = path.join(cwd, native, 'skills', rel);
    try {
      const stat = fs.lstatSync(file);
      if (!stat.isFile() || stat.isSymbolicLink() || fs.readFileSync(file, 'utf8') !== body)
        continue;
      fs.unlinkSync(file);
      for (const dir of [path.dirname(file), path.join(cwd, native, 'skills')]) {
        try {
          fs.rmdirSync(dir);
        } catch {
          break;
        }
      }
    } catch {
      // Missing, unreadable, or user-controlled entries are left untouched.
    }
  }
}

/** Install and point at spawn-agent and project-brief for supported ORDINARY
 * agents. Fleet Managers receive neither a pointer nor a native-discovery
 * copy. Pi has no MCP bridge, so advertising tool-driven skills there would be
 * decorative and misleading.
 */
export function installAgentCollaborationSkills(
  provider: AgentProvider,
  cwd: string,
  manager = false,
): string {
  removeLegacyNativeCopies(provider, cwd);
  if (provider === 'pi' || manager) return '';
  const root = agentCollaborationSkillsRoot(cwd);
  if (!installInto(cwd, root)) return '';
  return (
    'Workspacer provides two project skills: read ' +
    `${JSON.stringify(path.join(root, 'spawn-agent', 'SKILL.md'))} before spawning child agents, ` +
    `and ${JSON.stringify(path.join(root, 'project-brief', 'SKILL.md'))} before maintaining the project brief.`
  );
}
