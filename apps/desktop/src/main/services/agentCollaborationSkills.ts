/** App-owned ordinary-agent skills. Installed inside the session project so
 * children inherit them without modifying the user's personal skill library.
 * Collisions remain untouched; a versioned immutable copy supports harnesses
 * without a verified project-skills convention.
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

/** Install spawn-agent and project-brief for supported ordinary agents.
 * Claude and Codex discover project skills natively. Copilot and OpenCode get
 * a short pointer through their instruction channel. Pi has no MCP bridge, so
 * advertising tool-driven skills there would be decorative and misleading.
 */
export function installAgentCollaborationSkills(provider: AgentProvider, cwd: string): string {
  if (provider === 'pi') return '';
  const native = provider === 'claude' ? '.claude' : provider === 'codex' ? '.agents' : null;
  if (native && installInto(cwd, path.join(cwd, native, 'skills'))) return '';
  const root = agentCollaborationSkillsRoot(cwd);
  if (!installInto(cwd, root)) return '';
  return (
    'Workspacer provides two project skills: read ' +
    `${JSON.stringify(path.join(root, 'spawn-agent', 'SKILL.md'))} before spawning child agents, ` +
    `and ${JSON.stringify(path.join(root, 'project-brief', 'SKILL.md'))} before maintaining the project brief.`
  );
}
