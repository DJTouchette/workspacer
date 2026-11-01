// contracts/session-roster-cases.json is the SHARED roster fixture: hub-rs
// (services/hub-rs/src/services/roster.rs) loads the same vectors, so a
// desktop-provided `sessions.snapshots` and an engine hub's list agree on
// which sessions exist. See docs/session-roster.md.
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import { listed, starting, layoutSessionIds, type RosterContext } from './sessionRoster';

interface RosterCase {
  name: string;
  row: unknown;
  ctx: { open?: string[]; layout?: string[]; shells?: string[] };
  listed: boolean;
  starting: boolean;
}
const contract = JSON.parse(
  readFileSync(
    new URL('../../../../../contracts/session-roster-cases.json', import.meta.url),
    'utf8',
  ),
) as { version: number; now: string; cases: RosterCase[] };

describe('session roster — shared contract vectors', () => {
  it('carries cases', () => {
    expect(contract.version).toBe(1);
    expect(contract.cases.length).toBeGreaterThan(10);
  });
  for (const c of contract.cases) {
    it(c.name, () => {
      const ctx: RosterContext = {
        open: new Set(c.ctx.open ?? []),
        layout: new Set(c.ctx.layout ?? []),
        shells: new Set(c.ctx.shells ?? []),
      };
      expect(listed(c.row, ctx)).toBe(c.listed);
      expect(starting(c.row, ctx)).toBe(c.starting);
    });
  }
});

describe('session roster — desktop camelCase snapshots', () => {
  it('treats a store row with no mode as live unless it ended', () => {
    expect(listed({ sessionId: 'a', status: 'active' })).toBe(true);
    expect(listed({ sessionId: 'a', status: 'starting' })).toBe(true);
  });
  it('lists an ended store row only while it is open or curated', () => {
    const ended = { sessionId: 'a', status: 'ended', lastActivity: Date.now() };
    expect(listed(ended)).toBe(false);
    expect(listed(ended, { open: ['a'] })).toBe(true);
    expect(listed(ended, { layout: ['a'] })).toBe(true);
  });
  it('accepts an epoch-ms updatedAt but rejects a malformed one', () => {
    expect(listed({ sessionId: 'a', mode: 'input', updatedAt: 5 })).toBe(true);
    expect(listed({ sessionId: 'a', mode: 'input', updatedAt: 'x' })).toBe(false);
    expect(listed({ session_id: 'a', mode: 'input', updated_at: 5 })).toBe(false);
  });
  it('accepts plain arrays as context sets', () => {
    expect(listed({ session_id: 'a', mode: 'stopped' }, { layout: ['a'] })).toBe(true);
  });
});

describe('layoutSessionIds', () => {
  it('collects non-global agent and pane ids', () => {
    const ids = layoutSessionIds({
      data: {
        agents: [
          { global: true, sessionId: 'overview', lastSessionId: 'o2' },
          { sessionId: 'a', lastSessionId: 'b', tabs: [{ panes: [{ attachSessionId: 'c' }, {}] }] },
          null,
          { lastSessionId: '' },
        ],
      },
    });
    expect([...ids].sort()).toEqual(['a', 'b', 'c']);
  });
  it('is empty for a missing or malformed document', () => {
    expect(layoutSessionIds(null).size).toBe(0);
    expect(layoutSessionIds({ data: { agents: 'x' } }).size).toBe(0);
  });
});
