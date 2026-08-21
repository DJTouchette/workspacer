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
  const skillFile = (name: string) => path.join(home, '.claude', 'skills', name, 'SKILL.md');

  it('writes both /bearings and /stow with matching skill-name frontmatter', () => {
    installManagerSkills();
    const bearings = fs.readFileSync(skillFile('bearings'), 'utf8');
    const stow = fs.readFileSync(skillFile('stow'), 'utf8');
    expect(bearings).toMatch(/^---\nname: bearings\n/);
    expect(stow).toMatch(/^---\nname: stow\n/);
    // Load-bearing content: bearings is a read-only digest; stow routes + trims.
    expect(bearings).toContain('In flight');
    expect(bearings).toContain('must not change any brief');
    expect(stow).toContain('most specific home');
    expect(stow).toContain('.workspacer/brief.md');
    expect(stow).toContain('never rewrite a line the captain wrote');
  });

  it('is idempotent — a second install leaves identical content', () => {
    installManagerSkills();
    const body = fs.readFileSync(skillFile('stow'), 'utf8');
    installManagerSkills();
    expect(fs.readFileSync(skillFile('stow'), 'utf8')).toBe(body);
  });
});
