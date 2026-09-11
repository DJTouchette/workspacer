/**
 * installManagerSkills writes the Fleet Manager's invocable skills (/standup,
 * /checkpoint, /handoff) into ~/.claude/skills so a manager session can run
 * them. Twin of installSupervisorSkill; best-effort, idempotent,
 * content-addressed.
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
    // The installed skills remain bounded and preserve operational rules.
    expect(standup.length).toBeLessThan(2000);
    expect(checkpoint.length).toBeLessThan(3000);
    for (const term of [
      'In flight',
      'No spawning, polling or brief edits',
      'NEEDS A DECISION',
      'independent review',
      'lastMessage:true',
    ])
      expect(standup).toContain(term);
    for (const term of [
      'most specific home',
      '.workspacer/brief.md',
      'preserve',
      'User',
      'brief_archive',
      'keep:20',
      'brief.archive.md',
      'Do not delete history',
      'decide which are actually finished',
      'entriesInSection',
      'refuses lines over 4000',
      'rivet.learn',
      'atomic',
      'never a whole-file rewrite',
    ])
      expect(checkpoint).toContain(term);
  });

  it('writes /handoff with the succession contract a fresh manager needs', () => {
    installManagerSkills();
    const handoff = fs.readFileSync(skillFile('handoff'), 'utf8');
    expect(handoff).toMatch(/^---\nname: handoff\n/);
    // The load-bearing distinction: handoff must NOT reimplement checkpoint —
    // it RUNS it for the durable half and owns only the mid-flight half.
    expect(handoff).toContain('/checkpoint files what should OUTLIVE the session');
    expect(handoff).toContain('RESUME MID-FLIGHT');
    expect(handoff).toContain('do not reimplement it');
    expect(handoff).toContain('Pointers, never copies');
    // The four things a successor cannot re-derive from disk.
    expect(handoff).toContain('## In flight');
    expect(handoff).toContain('Told to:');
    expect(handoff).toContain('When it lands I owe it:');
    expect(handoff).toContain('## Waiting on the user');
    expect(handoff).toContain('## Established in conversation only');
    expect(handoff).toContain('## Next action');
    // Discovery: a sibling file PLUS a pointer in the always-read fleet brief.
    expect(handoff).toContain('.workspacer/handoff.md');
    expect(handoff).toContain('HANDOFF PENDING');
    // The verified wake truth: finish wakes route to the worker's PARENT
    // session, which the successor is not — so its FIRST action is to adopt
    // them, which re-points the routing key (claudeSessionStore.reparentChildren
    // via agents.reparent). Without this line the successor falls back to
    // reconciling ids by hand, which is what the tool exists to delete.
    expect(handoff).toContain("routed to a worker's PARENT SESSION");
    expect(handoff).toContain('adopt_workers({fromSessionId:');
    expect(handoff).toContain('toSessionId: "<your own session id>"');
    // Both ends of the call have to be findable from the file itself: the
    // predecessor's id is on the header line the template already writes.
    expect(handoff).toContain('Written by session:<your own session id>');
    // The reconciliation that survives is the bounded one — only the workers
    // that finished BEFORE the adoption — not the old open-ended polling loop.
    expect(handoff).toContain('finished BEFORE you adopted them');
    // A worker can talk back mid-task from any tier; the manager should say so
    // when it leaves its paper-trail instruction.
    expect(handoff).toContain('report_progress');
    // And the verified spawn truth: no role-less auto-successor, ever.
    expect(handoff).toContain('You cannot start the successor');
    expect(handoff).toContain('Terminate');
  });

  it('sweeps the superseded /bearings, /stow and /supervise dirs so no orphans linger', () => {
    // Simulate an earlier build having installed the old-named skills.
    // /supervise is the retired fleet-supervisor role: its SKILL.md is already
    // on every existing user's disk and nothing else would ever remove it, so
    // without this sweep a stale /supervise keeps appearing in the skill picker
    // and half-works by talking straight to claudemon on :7891.
    for (const old of ['bearings', 'stow', 'supervise']) {
      fs.mkdirSync(skillDir(old), { recursive: true });
      fs.writeFileSync(skillFile(old), 'old', 'utf8');
    }
    installManagerSkills();
    expect(fs.existsSync(skillDir('bearings'))).toBe(false);
    expect(fs.existsSync(skillDir('stow'))).toBe(false);
    expect(fs.existsSync(skillDir('supervise'))).toBe(false);
    expect(fs.existsSync(skillFile('standup'))).toBe(true);
    expect(fs.existsSync(skillFile('handoff'))).toBe(true);
  });

  it('is idempotent — a second install leaves identical content', () => {
    installManagerSkills();
    const body = fs.readFileSync(skillFile('checkpoint'), 'utf8');
    installManagerSkills();
    expect(fs.readFileSync(skillFile('checkpoint'), 'utf8')).toBe(body);
  });

  // A Fleet Manager on codex was previously left with NO slash commands: the
  // install was gated on Claude. Codex reads $CODEX_HOME/skills (else
  // ~/.codex/skills) and parses the identical SKILL.md format, so the fix is a
  // destination change — and the doctrine text must stay byte-identical, since
  // a per-provider copy is exactly what would drift.
  describe('per-provider destination', () => {
    const codexSkillFile = (name: string) => path.join(home, '.codex', 'skills', name, 'SKILL.md');

    it('writes the SAME skills into codex’s skills dir', () => {
      delete process.env.CODEX_HOME;
      installManagerSkills('claude');
      installManagerSkills('codex');
      for (const name of ['standup', 'checkpoint', 'handoff']) {
        expect(fs.existsSync(codexSkillFile(name))).toBe(true);
        expect(fs.readFileSync(codexSkillFile(name), 'utf8')).toBe(
          fs.readFileSync(skillFile(name), 'utf8'),
        );
      }
    });

    it('honours $CODEX_HOME', () => {
      const alt = path.join(home, 'alt-codex');
      process.env.CODEX_HOME = alt;
      try {
        installManagerSkills('codex');
        expect(fs.existsSync(path.join(alt, 'skills', 'standup', 'SKILL.md'))).toBe(true);
      } finally {
        delete process.env.CODEX_HOME;
      }
    });

    it('skips (loudly) a provider with no known skills directory', () => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      installManagerSkills('pi');
      expect(warn).toHaveBeenCalled();
      expect(fs.existsSync(path.join(home, '.pi'))).toBe(false);
      warn.mockRestore();
    });
  });
});
