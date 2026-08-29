/**
 * Per-harness profiles in the STORE: what a profile of each harness is allowed
 * to hold, enforced at write time.
 *
 * Two rules are load-bearing and pinned here rather than left to the UI:
 *
 *  1. A capability the harness does not have is stripped when the row is
 *     WRITTEN, not filtered when it is read — so `claude-profiles.json` never
 *     contains a value that looks like an opt-in and isn't. A weight on a
 *     Copilot profile could never fire (no usage-window signal exists), and a
 *     Library MCP selection outside Claude is a list nothing reads.
 *  2. The two new keys are OMITTED at their Claude defaults. Absent `provider`
 *     means Claude by contract, every profile in daily use predates the field,
 *     and the Go brain that round-trips this same file models neither key —
 *     emitting them on a Claude row would make the two providers answer
 *     `claude.profiles.list` with different shapes
 *     (contracts/claude-profiles-cases.json).
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const state = vi.hoisted(() => ({ dir: '' }));
vi.mock('./configService', () => ({ getConfigDir: () => state.dir }));

let sandboxes: string[] = [];
beforeEach(() => {
  sandboxes = [];
  state.dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-prof-prov-')));
  vi.resetModules();
});
afterEach(() => {
  for (const d of sandboxes) fs.rmSync(d, { recursive: true, force: true });
  fs.rmSync(state.dir, { recursive: true, force: true });
});

async function service(): Promise<typeof import('./claudeProfiles').claudeProfiles> {
  const mod = await import('./claudeProfiles');
  sandboxes.push(state.dir);
  return mod.claudeProfiles;
}

const onDisk = (): Record<string, unknown>[] =>
  JSON.parse(fs.readFileSync(path.join(state.dir, 'claude-profiles.json'), 'utf-8')).profiles;

describe('a Claude profile is untouched by the per-harness fields', () => {
  it('carries no provider and no preset key at all', async () => {
    const svc = await service();
    const p = svc.addProfile('Work', '~/.claude-work', ['--model', 'opus'], ['lib-1']);

    expect(p).not.toHaveProperty('provider');
    expect(p).not.toHaveProperty('preset');
    expect(p.mcpItemIds).toEqual(['lib-1']);
    expect(onDisk().find((r) => r.id === p.id)).not.toHaveProperty('provider');
  });

  it('keeps its failover weight — Claude reports a usage window', async () => {
    const svc = await service();
    const p = svc.addProfile('Work', '~/.claude-work', [], [], { weight: 7 });
    expect(p.weight).toBe(7);
  });
});

describe('a Codex profile', () => {
  it('records the harness and its native preset', async () => {
    const svc = await service();
    const p = svc.addProfile('Codex work', '~/.codex-work', [], [], {
      provider: 'codex',
      preset: 'work',
    });

    expect(p.provider).toBe('codex');
    expect(p.configDir).toBe('~/.codex-work');
    expect(p.preset).toBe('work');
  });

  it('keeps a failover weight — codex reports primary/secondary usage windows', async () => {
    const svc = await service();
    const p = svc.addProfile('Codex', '', [], [], { provider: 'codex', weight: 3 });
    expect(p.weight).toBe(3);
  });

  it('drops a Library MCP selection: it rides Claude’s --mcp-config alone', async () => {
    const svc = await service();
    const p = svc.addProfile('Codex', '', [], ['lib-1', 'lib-2'], { provider: 'codex' });
    expect(p.mcpItemIds).toEqual([]);
  });

  it('sanitizes the preset at write time, so the stored name is argv-safe', async () => {
    const svc = await service();
    const p = svc.addProfile('Codex', '', [], [], {
      provider: 'codex',
      preset: '--dangerously-bypass-approvals-and-sandbox',
    });
    expect(p).not.toHaveProperty('preset');
  });
});

describe('a Copilot profile', () => {
  it('has its failover weight forced to 0 — nothing could ever detect exhaustion', async () => {
    const svc = await service();
    const p = svc.addProfile('Copilot', '~/.copilot-work', [], [], {
      provider: 'copilot',
      weight: 9,
    });

    expect(p.provider).toBe('copilot');
    expect(p.weight).toBe(0);
    expect(onDisk().find((r) => r.id === p.id)!.weight).toBe(0);
  });

  it('has no preset key — copilot has no preset flag to apply one with', async () => {
    const svc = await service();
    const p = svc.addProfile('Copilot', '', [], [], { provider: 'copilot', preset: 'work' });
    expect(p).not.toHaveProperty('preset');
  });
});

describe('updateProfile re-judges the WHOLE row against the harness it ends on', () => {
  it('switching to copilot in the same call that sets a weight lands at 0', async () => {
    const svc = await service();
    const p = svc.addProfile('P', '', [], [], { provider: 'codex', weight: 4 });

    const updated = svc.updateProfile(p.id, { provider: 'copilot', weight: 6 })!;
    expect(updated.provider).toBe('copilot');
    expect(updated.weight).toBe(0);
  });

  it('switching back to Claude REMOVES the provider key, not just its value', async () => {
    const svc = await service();
    const p = svc.addProfile('P', '', [], [], { provider: 'codex', preset: 'work' });

    const updated = svc.updateProfile(p.id, { provider: 'claude' })!;
    expect(updated).not.toHaveProperty('provider');
    // The preset went with it: Claude has no preset flag.
    expect(updated).not.toHaveProperty('preset');
    expect(onDisk().find((r) => r.id === p.id)).not.toHaveProperty('preset');
  });

  it('survives a reload — the file is the source of truth, not the in-memory row', async () => {
    const first = await service();
    const p = first.addProfile('Codex', '~/.codex-work', ['-c', 'x=1'], [], {
      provider: 'codex',
      preset: 'work',
      weight: 2,
    });

    vi.resetModules();
    const reloaded = (await import('./claudeProfiles')).claudeProfiles;
    const got = reloaded.getProfile(p.id)!;
    expect(got.provider).toBe('codex');
    expect(got.preset).toBe('work');
    expect(got.weight).toBe(2);
    expect(got.extraArgs).toEqual(['-c', 'x=1']);
  });
});
