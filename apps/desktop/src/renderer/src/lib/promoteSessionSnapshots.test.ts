import { afterEach, describe, expect, it } from 'vitest';
import {
  omitSession,
  promoteSessionSnapshots,
  shouldEvictSession,
} from './promoteSessionSnapshots';
import { markSessionTerminated, resetTerminatedSessions } from './terminatedSessions';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';

function snapshot(overrides: Partial<ClaudeSessionSnapshot> = {}): ClaudeSessionSnapshot {
  return {
    sessionId: 's1',
    cwd: '/work',
    ptyId: 's1',
    status: 'active',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    pendingApproval: null,
    pendingQuestions: null,
    subagents: [],
    workflows: [],
    ambientState: 'idle',
    lastActivity: 1,
    totalToolCalls: 0,
    usage: null,
    ...overrides,
  } as ClaudeSessionSnapshot;
}

describe('promoteSessionSnapshots', () => {
  afterEach(() => {
    resetTerminatedSessions();
  });

  it('does NOT promote sessions the daemon reports as ended', () => {
    const sessions = [
      snapshot({ sessionId: 'live', status: 'active' }),
      snapshot({ sessionId: 'dead', status: 'ended' }),
    ];

    const { statusBySession, snapshotBySession } = promoteSessionSnapshots(sessions);

    // The live session is promoted...
    expect(statusBySession).toHaveProperty('live');
    expect(snapshotBySession).toHaveProperty('live');
    // ...but the ended session must be excluded, or it leaks forever (it never
    // ticks again, so the live-update cleanup can never evict it).
    expect(statusBySession).not.toHaveProperty('dead');
    expect(snapshotBySession).not.toHaveProperty('dead');
  });

  it('still excludes user-terminated sessions', () => {
    markSessionTerminated('killed');
    const sessions = [
      snapshot({ sessionId: 'killed', status: 'active' }),
      snapshot({ sessionId: 'ok', status: 'active' }),
    ];

    const { snapshotBySession } = promoteSessionSnapshots(sessions);

    expect(snapshotBySession).not.toHaveProperty('killed');
    expect(snapshotBySession).toHaveProperty('ok');
  });
});

describe('shouldEvictSession', () => {
  afterEach(() => {
    resetTerminatedSessions();
  });

  it('evicts a session the daemon reports as ended', () => {
    expect(shouldEvictSession('s1', 'ended')).toBe(true);
  });

  it('keeps a session that is merely idle or active', () => {
    expect(shouldEvictSession('s1', 'active')).toBe(false);
    expect(shouldEvictSession('s1', undefined)).toBe(false);
  });

  it('evicts a user-terminated session before the daemon calls it ended', () => {
    // Teardown ticks keep arriving after a terminate; without this the
    // snapshot is re-promoted and auto-adopt resurrects the closed card.
    markSessionTerminated('killed');
    expect(shouldEvictSession('killed', 'active')).toBe(true);
  });

  it('mirrors the exclusions promoteSessionSnapshots applies', () => {
    // The two paths must agree, or a session excluded at boot gets promoted by
    // the first live tick (or vice versa).
    markSessionTerminated('killed');
    const sessions = [
      snapshot({ sessionId: 'killed', status: 'active' }),
      snapshot({ sessionId: 'dead', status: 'ended' }),
      snapshot({ sessionId: 'ok', status: 'active' }),
    ];
    const { snapshotBySession } = promoteSessionSnapshots(sessions);
    for (const s of sessions) {
      expect(shouldEvictSession(s.sessionId, s.status)).toBe(!(s.sessionId in snapshotBySession));
    }
  });
});

describe('omitSession', () => {
  it('drops the session it was given', () => {
    expect(omitSession({ a: 1, b: 2 }, 'a')).toEqual({ b: 2 });
  });

  it('returns the very same object when the session is not there', () => {
    // Identity matters: these maps are React state, so a fresh object for a
    // session we never held would re-render every consumer on each teardown
    // tick of an unrelated session.
    const before = { a: 1 };
    expect(omitSession(before, 'missing')).toBe(before);
  });

  it('does not mutate the map it was given', () => {
    const before = { a: 1, b: 2 };
    const after = omitSession(before, 'a');
    expect(before).toEqual({ a: 1, b: 2 });
    expect(after).not.toBe(before);
  });

  it('empties out cleanly on the last entry', () => {
    expect(omitSession({ only: 1 }, 'only')).toEqual({});
  });
});
