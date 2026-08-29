import { describe, it, expect } from 'vitest';
import {
  pickFailoverProfile,
  profileFailoverPossible,
  windowExhausted,
  FAILOVER_BLOCK_MS,
} from './profileFailover';
import type { ClaudeProfile } from '../../../main/shared/ipcTypes';

const p = (over: Partial<ClaudeProfile> & { id: string }): ClaudeProfile => ({
  name: over.id,
  configDir: `~/.cfg-${over.id}`,
  extraArgs: [],
  isDefault: false,
  ...over,
});

const NONE: Record<string, boolean> = {};
const FRESH = () => new Map<string, number>();

describe('profileFailoverPossible', () => {
  it('is the capability table’s answer, not a name check', () => {
    expect(profileFailoverPossible('claude')).toBe(true);
    // Codex's primary/secondary windows land on the same fields the trigger reads.
    expect(profileFailoverPossible('codex')).toBe(true);
    // Copilot emits no RateLimits update, so nothing could ever fire.
    expect(profileFailoverPossible('copilot')).toBe(false);
    // No config root at all.
    expect(profileFailoverPossible('opencode')).toBe(false);
    expect(profileFailoverPossible('pi')).toBe(false);
    expect(profileFailoverPossible(undefined)).toBe(false);
  });
});

describe('pickFailoverProfile — harness scoping', () => {
  const profiles = [
    p({ id: 'claude-a', isDefault: true, weight: 0 }),
    p({ id: 'claude-b', weight: 5 }),
    p({ id: 'codex-a', provider: 'codex', weight: 9 }),
    p({ id: 'copilot-a', provider: 'copilot', weight: 9 }),
  ];

  it('never restarts a Claude session onto another harness’s config root', () => {
    const next = pickFailoverProfile(profiles, 'claude-a', NONE, FRESH(), 0, 'claude');
    // codex-a outweighs claude-b 9 to 5 — an unscoped rotation would have
    // pointed CLAUDE_CONFIG_DIR at a CODEX_HOME.
    expect(next?.id).toBe('claude-b');
  });

  it('rotates a Codex session onto a Codex profile', () => {
    const next = pickFailoverProfile(profiles, undefined, NONE, FRESH(), 0, 'codex');
    expect(next?.id).toBe('codex-a');
  });

  it('refuses outright for a harness with no usage-window signal', () => {
    expect(pickFailoverProfile(profiles, undefined, NONE, FRESH(), 0, 'copilot')).toBeNull();
  });

  it('refuses for a harness with no profiles at all', () => {
    expect(pickFailoverProfile(profiles, undefined, NONE, FRESH(), 0, 'opencode')).toBeNull();
  });

  it('defaults to Claude, which is what every pre-harness caller meant', () => {
    expect(pickFailoverProfile(profiles, 'claude-a', NONE, FRESH(), 0)?.id).toBe('claude-b');
  });

  it('treats a profile with no provider key as a Claude one', () => {
    // 'claude-b' carries no `provider` — it is one of the rows written before
    // the field existed, and it must still be a candidate.
    expect(profiles.find((x) => x.id === 'claude-b')?.provider).toBeUndefined();
    expect(pickFailoverProfile(profiles, 'claude-a', NONE, FRESH(), 0, 'claude')?.id).toBe(
      'claude-b',
    );
  });
});

describe('pickFailoverProfile — the rules that already held', () => {
  const profiles = [
    p({ id: 'a', isDefault: true, weight: 0 }),
    p({ id: 'b', weight: 5 }),
    p({ id: 'c', weight: 9 }),
  ];

  it('takes the heaviest, and never the one it is already on', () => {
    expect(pickFailoverProfile(profiles, 'a', NONE, FRESH(), 0)?.id).toBe('c');
    expect(pickFailoverProfile(profiles, 'c', NONE, FRESH(), 0)?.id).toBe('b');
  });

  it('skips an account with no login, but keeps an UNKNOWN one', () => {
    expect(pickFailoverProfile(profiles, 'a', { c: false }, FRESH(), 0)?.id).toBe('b');
    expect(pickFailoverProfile(profiles, 'a', {}, FRESH(), 0)?.id).toBe('c');
  });

  it('skips one that just hit its own limit, until the block expires', () => {
    const blocked = new Map([['c', 1_000]]);
    expect(pickFailoverProfile(profiles, 'a', NONE, blocked, 1_000)?.id).toBe('b');
    expect(
      pickFailoverProfile(profiles, 'a', NONE, blocked, 1_000 + FAILOVER_BLOCK_MS + 1)?.id,
    ).toBe('c');
  });

  it('excludes the DEFAULT row when the session named no profile', () => {
    const withWeightedDefault = [
      p({ id: 'a', isDefault: true, weight: 7 }),
      p({ id: 'b', weight: 5 }),
    ];
    expect(pickFailoverProfile(withWeightedDefault, undefined, NONE, FRESH(), 0)?.id).toBe('b');
  });

  it('returns null when there is nowhere to go', () => {
    expect(
      pickFailoverProfile([p({ id: 'a', isDefault: true })], 'a', NONE, FRESH(), 0),
    ).toBeNull();
  });
});

describe('windowExhausted', () => {
  it('fires just under 100 — the endpoint rounds', () => {
    expect(windowExhausted(99.5, 0)).toBe(true);
    expect(windowExhausted(0, 99.6)).toBe(true);
    expect(windowExhausted(99.4, 12)).toBe(false);
    expect(windowExhausted(undefined, undefined)).toBe(false);
  });
});
