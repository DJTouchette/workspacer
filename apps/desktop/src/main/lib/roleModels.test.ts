/**
 * Per-harness model resolution for the three INTERNAL ROLE spawns: the Fleet
 * Manager, the supervisor's digest workers, and the auto-title one-shot.
 *
 * The shape being pinned is the one the whole feature turns on — a legacy
 * single field that ships a CLAUDE id is honoured for claude and DROPPED for
 * codex, rather than being forwarded to a CLI that would refuse it. Falling
 * through to undefined ("the harness's own default") is the only answer valid
 * on every harness, so it is what an unresolvable value must produce.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

let mockConfig: Record<string, unknown>;
vi.mock('../services/configService', () => ({
  configService: { getConfig: () => mockConfig },
}));

const {
  perHarnessModel,
  resolveManagerModel,
  resolveSummarizerModel,
  resolveTitleModel,
  resolveSupervisorEffort,
  resolveManagerEffort,
} = await import('./roleModels');

beforeEach(() => {
  mockConfig = {};
});

describe('perHarnessModel — the shared formula', () => {
  it('prefers this harness’s own entry over the legacy field', () => {
    expect(perHarnessModel('codex', { codex: 'gpt-5.1-codex-max' }, 'sonnet')).toBe(
      'gpt-5.1-codex-max',
    );
  });

  it('falls back to the legacy field when THIS harness can serve it', () => {
    expect(perHarnessModel('claude', undefined, 'sonnet')).toBe('sonnet');
    expect(perHarnessModel('claude', { codex: 'gpt-5' }, 'sonnet')).toBe('sonnet');
  });

  it("DROPS a legacy field belonging to another harness — the 'sonnet' reaching codex bug", () => {
    expect(perHarnessModel('codex', undefined, 'sonnet')).toBeUndefined();
    expect(perHarnessModel('codex', {}, 'haiku')).toBeUndefined();
    expect(perHarnessModel('claude', undefined, 'gpt-5.1-codex-max')).toBeUndefined();
  });

  it('keeps a legacy id NOBODY claims — an unrecognized id is not a foreign one', () => {
    expect(perHarnessModel('codex', undefined, 'my-finetune-v3')).toBe('my-finetune-v3');
  });

  it('treats blank and whitespace-only values as unset, and trims what it returns', () => {
    expect(perHarnessModel('codex', { codex: '' }, 'gpt-5')).toBe('gpt-5');
    expect(perHarnessModel('codex', { codex: '   ' }, undefined)).toBeUndefined();
    expect(perHarnessModel('codex', { codex: '  gpt-5  ' })).toBe('gpt-5');
    expect(perHarnessModel('claude', undefined, '  ')).toBeUndefined();
    expect(perHarnessModel('claude', undefined, undefined)).toBeUndefined();
  });

  it('never leaks one harness’s entry to another', () => {
    const map = { claude: 'opus', codex: 'gpt-5.1-codex-max' };
    expect(perHarnessModel('claude', map)).toBe('opus');
    expect(perHarnessModel('codex', map)).toBe('gpt-5.1-codex-max');
    expect(perHarnessModel('opencode', map)).toBeUndefined();
  });
});

describe('resolveManagerModel', () => {
  it('reads agents.managerModels per harness', () => {
    mockConfig = { agents: { managerModels: { claude: 'opus[1m]', codex: 'gpt-5.1-codex-max' } } };
    expect(resolveManagerModel('claude')).toBe('opus[1m]');
    expect(resolveManagerModel('codex')).toBe('gpt-5.1-codex-max');
  });

  it('is undefined when unset — there is no legacy single field to inherit', () => {
    mockConfig = { agents: { managerProvider: 'codex' } };
    expect(resolveManagerModel('codex')).toBeUndefined();
    mockConfig = {};
    expect(resolveManagerModel('claude')).toBeUndefined();
  });

  it('does NOT fall back to claude.defaultModel — that is a claude-only field', () => {
    mockConfig = { claude: { defaultModel: 'opus[1m]' }, agents: {} };
    expect(resolveManagerModel('codex')).toBeUndefined();
  });
});

describe('resolveSummarizerModel', () => {
  it("honours the shipped 'sonnet' default for claude", () => {
    mockConfig = { supervisor: { summarizerModel: 'sonnet' } };
    expect(resolveSummarizerModel('claude')).toBe('sonnet');
  });

  it("refuses to hand that same 'sonnet' to a codex supervisor's digest workers", () => {
    mockConfig = { supervisor: { summarizerModel: 'sonnet' } };
    expect(resolveSummarizerModel('codex')).toBeUndefined();
    expect(resolveSummarizerModel('opencode')).toBeUndefined();
  });

  it('uses the per-harness entry when the user has picked one', () => {
    mockConfig = {
      supervisor: { summarizerModel: 'sonnet', summarizerModels: { codex: 'gpt-5' } },
    };
    expect(resolveSummarizerModel('codex')).toBe('gpt-5');
    expect(resolveSummarizerModel('claude')).toBe('sonnet');
  });
});

describe('resolveTitleModel', () => {
  it("honours the shipped 'haiku' default for claude and drops it for codex", () => {
    mockConfig = { agents: { autoTitle: { model: 'haiku' } } };
    expect(resolveTitleModel('claude')).toBe('haiku');
    expect(resolveTitleModel('codex')).toBeUndefined();
  });

  it('keeps several harnesses live at once — a mixed fleet titles each on its own', () => {
    mockConfig = {
      agents: {
        autoTitle: { model: 'haiku', models: { codex: 'gpt-5', opencode: 'anthropic/x-1' } },
      },
    };
    expect(resolveTitleModel('claude')).toBe('haiku');
    expect(resolveTitleModel('codex')).toBe('gpt-5');
    expect(resolveTitleModel('opencode')).toBe('anthropic/x-1');
    expect(resolveTitleModel('pi')).toBeUndefined();
  });

  it('is undefined with no autoTitle config at all', () => {
    mockConfig = {};
    expect(resolveTitleModel('claude')).toBeUndefined();
  });
});

/**
 * Reasoning EFFORT, the setting neither role had: a supervisor ran on whatever
 * its CLI defaults to with no way to raise it, and the pane implied otherwise
 * by showing nothing at all. Per-harness for the same reason as the models —
 * the ladders don't overlap (claude low..max, codex minimal..xhigh) — but with
 * NO legacy single field, so there is nothing to guard against here except
 * reading the wrong harness's entry.
 */
describe('resolveSupervisorEffort / resolveManagerEffort', () => {
  it('reads the entry for the harness being spawned, not another one', () => {
    mockConfig = {
      supervisor: { efforts: { codex: 'xhigh', claude: 'high' } },
      agents: { managerEfforts: { codex: 'medium' } },
    };
    expect(resolveSupervisorEffort('codex')).toBe('xhigh');
    expect(resolveSupervisorEffort('claude')).toBe('high');
    expect(resolveManagerEffort('codex')).toBe('medium');
    // Not configured on this harness = the harness's own default, the one
    // answer that is valid everywhere.
    expect(resolveManagerEffort('claude')).toBeUndefined();
  });

  it('treats absent, blank and whitespace as "the harness default"', () => {
    mockConfig = {};
    expect(resolveSupervisorEffort('claude')).toBeUndefined();
    mockConfig = { supervisor: { efforts: { claude: '' } }, agents: { managerEfforts: {} } };
    expect(resolveSupervisorEffort('claude')).toBeUndefined();
    expect(resolveManagerEffort('claude')).toBeUndefined();
    mockConfig = { supervisor: { efforts: { claude: '  high  ' } } };
    expect(resolveSupervisorEffort('claude')).toBe('high');
  });

  it('passes through a level the shipped ladder does not list', () => {
    // The ladders come from live catalogs; refusing an unrecognized level would
    // discard a deliberate choice on a newer CLI.
    mockConfig = { supervisor: { efforts: { codex: 'ultra' } } };
    expect(resolveSupervisorEffort('codex')).toBe('ultra');
  });
});
