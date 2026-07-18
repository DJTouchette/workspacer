import { afterEach, describe, expect, it } from 'vitest';
import { promoteSessionSnapshots } from './promoteSessionSnapshots';
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
