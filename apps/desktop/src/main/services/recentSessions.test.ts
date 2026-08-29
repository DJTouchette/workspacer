import { describe, it, expect, vi, afterEach } from 'vitest';
import { mergeRecentSessions, listLiveSessionIds } from './recentSessions';

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
  const hist = (over: Record<string, unknown> = {}) => ({
    sessionId: 's1',
    agentName: 'api-fix',
    model: 'claude-opus-4-8',
    costUSD: 1.25,
    inputTokens: 900,
    outputTokens: 100,
    ...over,
  });

  it('joins history names/model/cost/tokens by session id', () => {
    const out = mergeRecentSessions([row()], [hist()]);
    expect(out).toHaveLength(1);
    expect(out[0].name).toBe('api-fix');
    expect(out[0].model).toBe('claude-opus-4-8');
    expect(out[0].costUSD).toBe(1.25);
    expect(out[0].billedTokens).toBe(1000);
  });

  it('leaves name/model empty for sessions the history DB never saw, and reports cost/tokens as UNKNOWN rather than zero', () => {
    const out = mergeRecentSessions([row()], []);
    expect(out[0].name).toBe('');
    expect(out[0].model).toBe('');
    // Not 0: no row was found, so nothing was measured. A 0 here renders as a
    // confident "$0.00" on every surface downstream.
    expect(out[0].costUSD).toBeUndefined();
    expect(out[0].billedTokens).toBeUndefined();
  });

  it('reports an all-zero history row as unknown — cost_usd is DEFAULT 0 and never NULL, so a stored 0 is indistinguishable from never-written', () => {
    const out = mergeRecentSessions(
      [row()],
      [hist({ costUSD: 0, inputTokens: 0, outputTokens: 0 })],
    );
    expect(out[0].costUSD).toBeUndefined();
    expect(out[0].billedTokens).toBeUndefined();
    // The row itself is still known — only its usage is absent.
    expect(out[0].name).toBe('api-fix');
  });

  it('keeps tokens when only the cost is missing (a managed provider that reports tokens but no dollars)', () => {
    const out = mergeRecentSessions([row()], [hist({ costUSD: 0 })]);
    expect(out[0].costUSD).toBeUndefined();
    expect(out[0].billedTokens).toBe(1000);
  });

  it("treats a legacy empty provider as 'claude'", () => {
    const out = mergeRecentSessions(
      [row({ provider: '' }), row({ session_id: 's2', provider: 'codex' })],
      [],
    );
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

describe('listLiveSessionIds', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('returns the ids of every non-stopped session (unknown counts as live)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => [
          row({ session_id: 'live1', mode: 'input' }),
          row({ session_id: 'dead', mode: 'stopped' }),
          row({ session_id: 'live2', mode: 'unknown' }),
        ],
      }),
    );
    await expect(listLiveSessionIds()).resolves.toEqual(['live1', 'live2']);
  });

  it('returns null — not [] — when the daemon is unreachable, so boot reconciliation retries instead of stopping every agent', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('ECONNREFUSED')));
    await expect(listLiveSessionIds()).resolves.toBeNull();
  });

  it('returns null on a non-OK response or a non-array body', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false }));
    await expect(listLiveSessionIds()).resolves.toBeNull();
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) }));
    await expect(listLiveSessionIds()).resolves.toBeNull();
  });
});
