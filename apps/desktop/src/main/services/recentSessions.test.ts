import { describe, it, expect } from 'vitest';
import { mergeRecentSessions } from './recentSessions';

const row = (over: Record<string, unknown> = {}) => ({
  session_id: 's1',
  cwd: '/home/u/proj',
  mode: 'stopped',
  provider: 'claude',
  transport: 'pty' as const,
  updated_at: '2026-07-18T10:00:00Z',
  started_at: '2026-07-18T09:00:00Z',
  archived: false,
  ...over,
});

describe('mergeRecentSessions', () => {
  it('joins history names/model/cost by session id', () => {
    const out = mergeRecentSessions(
      [row()],
      [{ sessionId: 's1', agentName: 'api-fix', model: 'claude-opus-4-8', costUSD: 1.25 }],
    );
    expect(out).toHaveLength(1);
    expect(out[0].name).toBe('api-fix');
    expect(out[0].model).toBe('claude-opus-4-8');
    expect(out[0].costUSD).toBe(1.25);
  });

  it('leaves name/model empty for sessions the history DB never saw', () => {
    const out = mergeRecentSessions([row()], []);
    expect(out[0].name).toBe('');
    expect(out[0].model).toBe('');
    expect(out[0].costUSD).toBe(0);
  });

  it("treats a legacy empty provider as 'claude'", () => {
    const out = mergeRecentSessions([row({ provider: '' }), row({ session_id: 's2', provider: 'codex' })], []);
    expect(out.map((s) => s.provider).sort()).toEqual(['claude', 'codex']);
  });

  it('drops subagent sidechain rows (agent-*)', () => {
    const out = mergeRecentSessions([row({ session_id: 'agent-abc' }), row()], []);
    expect(out.map((s) => s.sessionId)).toEqual(['s1']);
  });

  it('sorts newest first and survives unparseable timestamps', () => {
    const out = mergeRecentSessions(
      [
        row({ session_id: 'old', updated_at: '2026-07-01T00:00:00Z' }),
        row({ session_id: 'bad', updated_at: 'not-a-date' }),
        row({ session_id: 'new', updated_at: '2026-07-18T12:00:00Z' }),
      ],
      [],
    );
    expect(out.map((s) => s.sessionId)).toEqual(['new', 'old', 'bad']);
    expect(out[2].updatedAt).toBe(0);
  });

  it('carries mode, transport, and the archived flag through', () => {
    const out = mergeRecentSessions(
      [row({ mode: 'input', transport: 'stream', archived: true })],
      [],
    );
    expect(out[0].mode).toBe('input');
    expect(out[0].transport).toBe('stream');
    expect(out[0].archived).toBe(true);
  });
});
