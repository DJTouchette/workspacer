/**
 * Which HARNESS a role spawn runs on when the caller named none.
 *
 * The bug this closes: `supervisor.provider` was persisted by Settings and read
 * by exactly one renderer component, so a supervisor started any other way
 * (hub bus, phone, a hub job, a respawn) fell through to 'claude' while
 * Settings said codex — and a silently-Claude supervisor looks exactly like a
 * working one. Pinned here rather than only at the call sites because the whole
 * point is that ONE formula serves every entry point.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

let mockConfig: Record<string, unknown>;
vi.mock('../services/configService', () => ({
  configService: { getConfig: () => mockConfig },
}));

const { resolveSupervisorProvider, resolveManagerProvider, resolveSpawnProvider } =
  await import('./roleProviders');

beforeEach(() => {
  mockConfig = {};
});

describe('resolveSupervisorProvider / resolveManagerProvider', () => {
  it('returns the configured harness', () => {
    mockConfig = { supervisor: { provider: 'codex' }, agents: { managerProvider: 'codex' } };
    expect(resolveSupervisorProvider()).toBe('codex');
    expect(resolveManagerProvider()).toBe('codex');
  });

  it('defaults to claude when the setting is absent, blank, or unknown', () => {
    expect(resolveSupervisorProvider()).toBe('claude');
    mockConfig = { supervisor: { provider: '   ' } };
    expect(resolveSupervisorProvider()).toBe('claude');
    // A hand-edited config naming a harness we do not speak must not reach an
    // adapter that has no idea what it is.
    mockConfig = { supervisor: { provider: 'gpt6' } };
    expect(resolveSupervisorProvider()).toBe('claude');
    mockConfig = { agents: { managerProvider: 'gpt6' } };
    expect(resolveManagerProvider()).toBe('claude');
  });

  it('tolerates whitespace around a real id', () => {
    mockConfig = { supervisor: { provider: ' opencode ' } };
    expect(resolveSupervisorProvider()).toBe('opencode');
  });
});

describe('resolveSpawnProvider — what the two spawn funnels call', () => {
  beforeEach(() => {
    mockConfig = {
      supervisor: { provider: 'opencode' },
      agents: { managerProvider: 'codex' },
    };
  });

  it('honours the role settings for a role spawn that names no provider', () => {
    expect(resolveSpawnProvider({ supervisor: true })).toBe('opencode');
    expect(resolveSpawnProvider({ manager: true })).toBe('codex');
  });

  it('lets an explicit provider win — a per-launch pick is not a config default', () => {
    expect(resolveSpawnProvider({ supervisor: true, provider: 'claude' })).toBe('claude');
    expect(resolveSpawnProvider({ manager: true, provider: 'claude' })).toBe('claude');
  });

  it('leaves a plain worker on claude — these settings are for the two roles only', () => {
    expect(resolveSpawnProvider({})).toBe('claude');
    expect(resolveSpawnProvider({ mcpFacade: true } as never)).toBe('claude');
  });

  it('reads a both-flags request as a manager — that is what the Fleet Manager is', () => {
    // The manager spawns with supervisor-style wake wiring; the setting the
    // user set FOR IT is the manager's.
    expect(resolveSpawnProvider({ supervisor: true, manager: true })).toBe('codex');
  });
});
