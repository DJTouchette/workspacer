/**
 * The Fleet Manager's invocable skills (/standup, /checkpoint, /handoff): their
 * bundled text, and the personal-dir install kept for harnesses with no
 * per-session skill loading (Copilot). Claude and Codex take the same text as
 * the session-only `workspacer-fleet` plugin, so a personal copy is refused.
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

import { installManagerSkills, managerSkillBody } from './managerSkills';

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
  const skillDir = (name: string) => path.join(home, '.copilot', 'skills', name);
  const skillFile = (name: string) => path.join(skillDir(name), 'SKILL.md');

  it('writes both /standup and /checkpoint with matching skill-name frontmatter', () => {
    installManagerSkills('copilot');
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
    installManagerSkills('copilot');
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
    installManagerSkills('copilot');
    expect(fs.existsSync(skillDir('bearings'))).toBe(false);
    expect(fs.existsSync(skillDir('stow'))).toBe(false);
    expect(fs.existsSync(skillDir('supervise'))).toBe(false);
    expect(fs.existsSync(skillFile('standup'))).toBe(true);
    expect(fs.existsSync(skillFile('handoff'))).toBe(true);
  });

  it('is idempotent — a second install leaves identical content', () => {
    installManagerSkills('copilot');
    const body = fs.readFileSync(skillFile('checkpoint'), 'utf8');
    installManagerSkills('copilot');
    expect(fs.readFileSync(skillFile('checkpoint'), 'utf8')).toBe(body);
  });

  it('installs exactly the bundled plugin text', () => {
    installManagerSkills('copilot');
    for (const name of ['standup', 'checkpoint', 'handoff'] as const) {
      expect(fs.readFileSync(skillFile(name), 'utf8')).toBe(managerSkillBody(name));
    }
  });

  it.each(['claude', 'codex'] as const)(
    'refuses a personal copy for %s, which takes the skills per session',
    (provider) => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      installManagerSkills(provider);
      expect(warn).toHaveBeenCalled();
      expect(() => installManagerSkills(provider, true)).toThrow();
      expect(fs.existsSync(path.join(home, '.claude'))).toBe(false);
      expect(fs.existsSync(path.join(home, '.codex'))).toBe(false);
      warn.mockRestore();
    },
  );

  it('skips (loudly) a provider with no known skills directory', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    installManagerSkills('pi');
    expect(warn).toHaveBeenCalled();
    expect(fs.existsSync(path.join(home, '.pi'))).toBe(false);
    warn.mockRestore();
  });
});
