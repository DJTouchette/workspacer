/**
 * Which HARNESS a manager spawn runs on when the caller named none.
 *
 * The bug this closes: `agents.managerProvider` was persisted by Settings and
 * read by exactly one renderer component, so a manager started any other way
 * (hub bus, phone, a hub job, a respawn) fell through to 'claude' while
 * Settings said codex — and a silently-Claude manager looks exactly like a
 * working one. Pinned here rather than only at the call sites because the whole
 * point is that ONE formula serves every entry point.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

let mockConfig: Record<string, unknown>;
vi.mock('../services/configService', () => ({
  configService: { getConfig: () => mockConfig },
}));

const { resolveManagerProvider, resolveSpawnProvider } = await import('./roleProviders');

beforeEach(() => {
  mockConfig = {};
});

describe('resolveManagerProvider', () => {
  it('returns the configured harness', () => {
    mockConfig = { agents: { managerProvider: 'codex' } };
    expect(resolveManagerProvider()).toBe('codex');
  });

  it('defaults to claude when the setting is absent, blank, or unknown', () => {
    expect(resolveManagerProvider()).toBe('claude');
    mockConfig = { agents: { managerProvider: '   ' } };
    expect(resolveManagerProvider()).toBe('claude');
    // A hand-edited config naming a harness we do not speak must not reach an
    // adapter that has no idea what it is.
    mockConfig = { agents: { managerProvider: 'gpt6' } };
    expect(resolveManagerProvider()).toBe('claude');
  });

  it('tolerates whitespace around a real id', () => {
    mockConfig = { agents: { managerProvider: ' opencode ' } };
    expect(resolveManagerProvider()).toBe('opencode');
  });
});

describe('resolveSpawnProvider — what the two spawn funnels call', () => {
  beforeEach(() => {
    mockConfig = { agents: { managerProvider: 'codex' } };
  });

  it('honours the manager setting for a manager spawn that names no provider', () => {
    expect(resolveSpawnProvider({ manager: true })).toBe('codex');
  });

  it('lets an explicit provider win — a per-launch pick is not a config default', () => {
    expect(resolveSpawnProvider({ manager: true, provider: 'claude' })).toBe('claude');
  });

  it('leaves a plain worker on claude — this setting is for the manager only', () => {
    expect(resolveSpawnProvider({})).toBe('claude');
    expect(resolveSpawnProvider({ mcpFacade: true } as never)).toBe('claude');
  });
});
