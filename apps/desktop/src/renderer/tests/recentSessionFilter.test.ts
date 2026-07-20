import { describe, it, expect } from 'vitest';
import { filterResumableSessions, recentSessionLabel } from '../src/lib/recentSessionFilter';
import type { RecentAgentSession } from '../../main/shared/ipcTypes';

const sess = (id: string, over: Partial<RecentAgentSession> = {}): RecentAgentSession => ({
  sessionId: id,
  provider: 'claude',
  cwd: '/x',
  mode: 'stopped',
  transport: 'pty',
  archived: false,
  updatedAt: 1,
  startedAt: 1,
  name: '',
  title: '',
  model: '',
  costUSD: 0,
  ...over,
});

const agent = (over: Record<string, unknown> = {}) =>
  ({
    id: 'a1',
    name: 'A',
    cwd: '/x',
    activeTabId: 't1',
    tabs: [{ id: 't1', title: 'T', activePaneId: 'p1', panes: [{ id: 'p1', type: 'claude' }] }],
    ...over,
  }) as any;

describe('filterResumableSessions', () => {
  it('excludes sessions already represented in the layout', () => {
    const agents = [
      agent({ sessionId: 'live' }),
      agent({ id: 'a2', lastSessionId: 'respawnable' }),
      agent({
        id: 'a3',
        tabs: [
          {
            id: 't1',
            title: 'T',
            activePaneId: 'p1',
            panes: [
              { id: 'p1', type: 'claude', resumeSessionId: 'resuming' },
              { id: 'p2', type: 'claude', attachSessionId: 'attached' },
            ],
          },
        ],
      }),
    ];
    const out = filterResumableSessions(
      [sess('live'), sess('respawnable'), sess('resuming'), sess('attached'), sess('free')],
      agents,
      { p9: 'mapped' },
    );
    expect(out.map((s) => s.sessionId)).toEqual(['free']);
  });

  it('excludes sessions in the ptyMapping', () => {
    const out = filterResumableSessions([sess('mapped'), sess('free')], [], { p1: 'mapped' });
    expect(out.map((s) => s.sessionId)).toEqual(['free']);
  });

  it('only offers stopped sessions — a live row belongs to another client', () => {
    const out = filterResumableSessions([sess('busy', { mode: 'responding' }), sess('ok')], [], {});
    expect(out.map((s) => s.sessionId)).toEqual(['ok']);
  });

  it('drops rows without a cwd (nothing to respawn into)', () => {
    const out = filterResumableSessions([sess('nocwd', { cwd: '' }), sess('ok')], [], {});
    expect(out.map((s) => s.sessionId)).toEqual(['ok']);
  });

  it('caps the list at the limit', () => {
    const many = Array.from({ length: 30 }, (_, i) => sess(`s${i}`));
    expect(filterResumableSessions(many, [], {})).toHaveLength(20);
    expect(filterResumableSessions(many, [], {}, 5)).toHaveLength(5);
  });
});

describe('recentSessionLabel', () => {
  it('prefers an explicitly-given name over the auto title', () => {
    const s = sess('s1', { cwd: '/work/proj', name: 'my renamed agent', title: 'Fix the bug' });
    expect(recentSessionLabel(s)).toBe('my renamed agent');
  });

  it('treats a name equal to the cwd basename as the spawn default, not explicit', () => {
    const s = sess('s1', { cwd: '/work/proj', name: 'proj', title: 'Fix the sidebar cards' });
    expect(recentSessionLabel(s)).toBe('Fix the sidebar cards');
  });

  it('falls back name → dirname when there is no title', () => {
    expect(recentSessionLabel(sess('s1', { cwd: '/work/proj', name: 'proj' }))).toBe('proj');
    expect(recentSessionLabel(sess('s1', { cwd: '/work/proj' }))).toBe('proj');
  });

  it('falls back to the session id when nothing else exists', () => {
    expect(recentSessionLabel(sess('abcdef1234', { cwd: '' }))).toBe('abcdef12');
  });
});
