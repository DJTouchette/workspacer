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
    // Load-bearing content: standup is a read-only digest; checkpoint routes + trims.
    expect(standup).toContain('In flight');
    expect(standup).toContain('must not change any brief');
    // A worker's mid-task report arrives as a [fleet] wake in this very
    // conversation — the digest reads it there instead of re-asking the worker.
    expect(standup).toContain('report_progress');
    expect(standup).toContain('NEEDS A DECISION');
    expect(checkpoint).toContain('most specific home');
    expect(checkpoint).toContain('.workspacer/brief.md');
    expect(checkpoint).toContain('inspect-then-edit');
    expect(checkpoint).toContain('## User');
    // Pruning is COLD ARCHIVAL, not deletion — the overflow moves to the
    // archive so the brief stays short but the history survives.
    expect(checkpoint).toContain('20 newest');
    expect(checkpoint).toContain('.workspacer/brief.archive.md');
    // And it goes through the CAPABILITY, not a shell. A skill body that only
    // describes the destination is what produced three differently worded
    // archive headings and four .bak files in one morning.
    expect(checkpoint).toContain('brief_archive({project, section: "Recently", keep: 20})');
    expect(checkpoint).toContain('do not do this with');
    // The judgement stays with the model: which lines are stale is the part a
    // schema is bad at, and it was right when a model did it by hand.
    expect(checkpoint).toContain('judgement only you can make');
    // The two things brief_append reports back, so an over-budget section and a
    // refused line are both acted on rather than discovered by failing.
    expect(checkpoint).toContain('entriesInSection');
    expect(checkpoint).toContain('REFUSED');
    // A durable project finding routes to rivet.learn when the project uses it.
    expect(checkpoint).toContain('rivet.learn');
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
