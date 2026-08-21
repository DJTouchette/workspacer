/**
 * installManagerSkills writes the Fleet Manager's invocable skills (/bearings,
 * /stow) into ~/.claude/skills so a manager session can run them. Twin of
 * installSupervisorSkill; best-effort, idempotent, content-addressed.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

// os.homedir is where the skills land; point it at a scratch dir per test.
// (ESM can't spyOn a module export, so mock the module and swap the return —
//  the holder is hoisted so the mock factory can close over it.)
const holder = vi.hoisted(() => ({ home: '' }));
vi.mock('os', async (importOriginal) => {
  const actual = await importOriginal<typeof import('os')>();
  return { ...actual, homedir: () => holder.home || actual.homedir() };
});

import { installManagerSkills } from './managerSkills';

let home: string;
beforeEach(() => {
  home = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-mgr-skills-'));
  holder.home = home;
});
afterEach(() => {
  fs.rmSync(home, { recursive: true, force: true });
  holder.home = '';
});

describe('installManagerSkills', () => {
  const skillDir = (name: string) => path.join(home, '.claude', 'skills', name);
  const skillFile = (name: string) => path.join(skillDir(name), 'SKILL.md');

  it('writes both /standup and /checkpoint with matching skill-name frontmatter', () => {
    installManagerSkills();
    const standup = fs.readFileSync(skillFile('standup'), 'utf8');
    const checkpoint = fs.readFileSync(skillFile('checkpoint'), 'utf8');
    expect(standup).toMatch(/^---\nname: standup\n/);
    expect(checkpoint).toMatch(/^---\nname: checkpoint\n/);
    // Load-bearing content: standup is a read-only digest; checkpoint routes + trims.
    expect(standup).toContain('In flight');
    expect(standup).toContain('must not change any brief');
    expect(checkpoint).toContain('most specific home');
    expect(checkpoint).toContain('.workspacer/brief.md');
    expect(checkpoint).toContain('inspect-then-edit');
    expect(checkpoint).toContain('## User');
    // Pruning is COLD ARCHIVAL, not deletion — the overflow moves to the
    // archive so the brief stays short but the history survives.
    expect(checkpoint).toContain('~20 newest');
    expect(checkpoint).toContain('.workspacer/brief.archive.md');
    expect(checkpoint).toContain('Do NOT delete the overflow');
    // A durable project finding routes to rivet.learn when the project uses it.
    expect(checkpoint).toContain('rivet.learn');
  });

  it('sweeps the superseded /bearings and /stow dirs so no orphans linger', () => {
    // Simulate an earlier build having installed the old-named skills.
    for (const old of ['bearings', 'stow']) {
      fs.mkdirSync(skillDir(old), { recursive: true });
      fs.writeFileSync(skillFile(old), 'old', 'utf8');
    }
    installManagerSkills();
    expect(fs.existsSync(skillDir('bearings'))).toBe(false);
    expect(fs.existsSync(skillDir('stow'))).toBe(false);
    expect(fs.existsSync(skillFile('standup'))).toBe(true);
  });

  it('is idempotent — a second install leaves identical content', () => {
    installManagerSkills();
    const body = fs.readFileSync(skillFile('checkpoint'), 'utf8');
    installManagerSkills();
    expect(fs.readFileSync(skillFile('checkpoint'), 'utf8')).toBe(body);
  });
});
