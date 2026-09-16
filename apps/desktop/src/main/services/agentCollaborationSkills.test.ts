import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  AGENT_COLLABORATION_SKILL_NAMES,
  agentCollaborationSkillsRoot,
  installAgentCollaborationSkills,
} from './agentCollaborationSkills';

const roots: string[] = [];
function project(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-agent-skills-'));
  roots.push(dir);
  return dir;
}

afterEach(() => {
  for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe('ordinary-agent collaboration skills', () => {
  it.each(['claude', 'codex', 'copilot', 'opencode'] as const)(
    'gives ordinary %s sessions a pointer to immutable app-owned skills',
    (provider) => {
      const cwd = project();
      const note = installAgentCollaborationSkills(provider, cwd);
      for (const name of AGENT_COLLABORATION_SKILL_NAMES) {
        const file = path.join(agentCollaborationSkillsRoot(cwd), name, 'SKILL.md');
        const body = fs.readFileSync(file, 'utf8');
        expect(body).toContain(`name: ${name}`);
        expect(note).toContain(file);
      }
      expect(fs.existsSync(path.join(cwd, '.claude', 'skills'))).toBe(false);
      expect(fs.existsSync(path.join(cwd, '.agents', 'skills'))).toBe(false);
    },
  );

  it.each(['claude', 'codex', 'copilot', 'opencode'] as const)(
    'gives %s Fleet Managers neither a pointer nor native discovery',
    (provider) => {
      const cwd = project();
      expect(installAgentCollaborationSkills(provider, cwd, true)).toBe('');
      expect(fs.existsSync(path.join(cwd, '.workspacer'))).toBe(false);
      expect(fs.existsSync(path.join(cwd, '.claude', 'skills'))).toBe(false);
      expect(fs.existsSync(path.join(cwd, '.agents', 'skills'))).toBe(false);
    },
  );

  it('does not advertise decorative skills to Pi, which has no MCP bridge', () => {
    const cwd = project();
    expect(installAgentCollaborationSkills('pi', cwd)).toBe('');
    expect(fs.existsSync(path.join(cwd, '.workspacer'))).toBe(false);
    expect(fs.existsSync(path.join(cwd, '.claude'))).toBe(false);
    expect(fs.existsSync(path.join(cwd, '.agents'))).toBe(false);
  });

  it('preserves a user-owned collision instead of overwriting it', () => {
    const cwd = project();
    const file = path.join(cwd, '.claude', 'skills', 'spawn-agent', 'SKILL.md');
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, 'user-owned\n');
    const note = installAgentCollaborationSkills('claude', cwd);
    expect(fs.readFileSync(file, 'utf8')).toBe('user-owned\n');
    expect(note).toContain(agentCollaborationSkillsRoot(cwd));
  });

  it('removes only an exact stale app-owned native copy before a manager starts', () => {
    const cwd = project();
    installAgentCollaborationSkills('claude', cwd);
    const body = fs.readFileSync(
      path.join(agentCollaborationSkillsRoot(cwd), 'spawn-agent', 'SKILL.md'),
      'utf8',
    );
    const file = path.join(cwd, '.claude', 'skills', 'spawn-agent', 'SKILL.md');
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, body);

    expect(installAgentCollaborationSkills('claude', cwd, true)).toBe('');
    expect(fs.existsSync(file)).toBe(false);
  });
});
