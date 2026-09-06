/** Repo-owned skill assets, installed in the session PROJECT only. No writes to
 * personal skills/config. Collisions (including edited generated files) remain
 * untouched; an immutable product copy supports a short instruction pointer.
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { createHash } from 'crypto';
import type { AgentProvider } from './agentProviders';
import files from './responseCardSkill.generated.json';

export const RESPONSE_CARD_SKILL_NAME = 'workspacer-response-cards';
const version = createHash('sha256').update(JSON.stringify(files)).digest('hex').slice(0, 16);

export function responseCardSkillRoot(cwd: string): string {
  return path.join(cwd, '.workspacer', 'skills', version, RESPONSE_CARD_SKILL_NAME);
}

// Refuse symlinks at every destination component. Exclusive file creation never
// overwrites a user-owned file; compare ALL existing files before writing any.
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
    if (fs.existsSync(path.join(dir, '.disabled'))) return false;
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

/** Returns only the instruction-channel fallback, never the entire contract.
 * Claude's project .claude/skills and Codex's .agents/skills are native roots.
 * Other adapters get a file pointer; this makes no claim they lack skills.
 */
export function installResponseCardSkill(provider: AgentProvider, cwd: string): string {
  const native = provider === 'claude' ? '.claude' : provider === 'codex' ? '.agents' : null;
  if (
    native &&
    fs.existsSync(path.join(cwd, native, 'skills', RESPONSE_CARD_SKILL_NAME, '.disabled'))
  )
    return '';
  if (native && installInto(cwd, path.join(cwd, native, 'skills', RESPONSE_CARD_SKILL_NAME)))
    return '';
  const root = responseCardSkillRoot(cwd);
  if (!installInto(cwd, root)) return '';
  return `For a structured visual answer in Workspacer chat, read ${JSON.stringify(path.join(root, 'SKILL.md'))} to use inline HTML response cards. Keep your written summary and any required result/escalation contract.`;
}
