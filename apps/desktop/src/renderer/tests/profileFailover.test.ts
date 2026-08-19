/**
 * pickFailoverProfile — the "cycle through until one works" ordering. Wrong
 * answers here either bounce a session between two exhausted accounts or
 * strand it on a limited one with a healthy backup sitting idle.
 */

import { describe, it, expect } from 'vitest';
import {
  pickFailoverProfile,
  windowExhausted,
  FAILOVER_BLOCK_MS,
} from '../src/lib/profileFailover';
import type { ClaudeProfile } from '../../main/shared/ipcTypes';

const prof = (id: string, weight: number, isDefault = false): ClaudeProfile => ({
  id,
  name: id,
  configDir: id === 'default' ? '' : `/cfg/${id}`,
  extraArgs: [],
  mcpItemIds: [],
  isDefault,
  weight,
});

const NOW = 1_000_000_000;

describe('pickFailoverProfile', () => {
  it('picks the heaviest signed-in candidate that is not the current profile', () => {
    const profiles = [prof('default', 0, true), prof('work', 5), prof('backup', 9)];
    const pick = pickFailoverProfile(profiles, undefined, { backup: true, work: true }, new Map(), NOW);
    expect(pick?.id).toBe('backup');
  });

  it('weight 0 profiles never join the rotation — weights ARE the setting', () => {
    const profiles = [prof('default', 0, true), prof('work', 0)];
    expect(pickFailoverProfile(profiles, undefined, {}, new Map(), NOW)).toBeNull();
  });

  it('an undefined current profile means the DEFAULT row, which is excluded', () => {
    // Even a weighted default must not be "switched to" from itself.
    const profiles = [prof('default', 10, true), prof('work', 5)];
    const pick = pickFailoverProfile(profiles, undefined, {}, new Map(), NOW);
    expect(pick?.id).toBe('work');
  });

  it('a signed-out account is skipped; an UNKNOWN status is kept', () => {
    const profiles = [prof('default', 0, true), prof('a', 9), prof('b', 5)];
    const pick = pickFailoverProfile(profiles, undefined, { a: false }, new Map(), NOW);
    expect(pick?.id).toBe('b');
    // No status at all → still a candidate (the pane's sign-in banner covers it).
    const pick2 = pickFailoverProfile(profiles, undefined, {}, new Map(), NOW);
    expect(pick2?.id).toBe('a');
  });

  it('a recently exhausted stop is skipped, and re-eligible after the block window', () => {
    const profiles = [prof('default', 0, true), prof('a', 9), prof('b', 5)];
    const blocked = new Map([['a', NOW - 1000]]);
    expect(pickFailoverProfile(profiles, undefined, {}, blocked, NOW)?.id).toBe('b');
    expect(
      pickFailoverProfile(profiles, undefined, {}, blocked, NOW + FAILOVER_BLOCK_MS + 1)?.id,
    ).toBe('a');
  });

  it('cycles: from one weighted profile to the next, never back to itself', () => {
    const profiles = [prof('default', 0, true), prof('a', 9), prof('b', 5)];
    expect(pickFailoverProfile(profiles, 'a', {}, new Map(), NOW)?.id).toBe('b');
    expect(pickFailoverProfile(profiles, 'b', {}, new Map(), NOW)?.id).toBe('a');
  });

  it('equal weights break ties by name for a stable order', () => {
    const profiles = [prof('zed', 5), prof('alpha', 5)];
    expect(pickFailoverProfile(profiles, undefined, {}, new Map(), NOW)?.id).toBe('alpha');
  });
});

describe('windowExhausted', () => {
  it('trips at ~100 on either window and not below', () => {
    expect(windowExhausted(99.6, 0)).toBe(true);
    expect(windowExhausted(0, 100)).toBe(true);
    expect(windowExhausted(98, 98)).toBe(false);
    expect(windowExhausted(undefined, undefined)).toBe(false);
  });
});
