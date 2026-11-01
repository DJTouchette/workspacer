import { beforeEach, describe, expect, it, vi } from 'vitest';

// Collaborators of the provider: claudemon readers, the hub, the shared open
// set, the history DB, transcript titles and the live store.
const fetchDaemonHistory = vi.fn();
const forgetDaemonSession = vi.fn();
const getDaemonSessionRow = vi.fn();
const listDaemonSessionRows = vi.fn();
vi.mock('./recentSessions', () => ({
  fetchDaemonHistory: (q: unknown) => fetchDaemonHistory(q),
  forgetDaemonSession: (id: string) => forgetDaemonSession(id),
  getDaemonSessionRow: (id: string) => getDaemonSessionRow(id),
  listDaemonSessionRows: () => listDaemonSessionRows(),
}));
const callHub = vi.fn();
vi.mock('./hubClient', () => ({ callHub: (m: string, p: unknown) => callHub(m, p) }));
const openSessionIds = vi.fn(async () => new Set<string>());
vi.mock('./sessionOpenCache', () => ({ openSessionIds: () => openSessionIds() }));
const recent = vi.fn(() => [] as unknown[]);
vi.mock('./sessionHistory', () => ({ sessionHistory: { recent: (n: number) => recent(n) } }));
const titleForSession = vi.fn(async () => undefined as string | undefined);
vi.mock('./sessionTitles', () => ({ titleForSession: (r: unknown) => titleForSession(r) }));
const getSnapshot = vi.fn(() => null as unknown);
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { getSnapshot: (id: string) => getSnapshot(id) },
}));

import {
  fillEvictedOpenRows,
  forgetSession,
  historyQuery,
  historyRows,
  listSessionHistory,
  resetEvictedOpenRowsForTests,
} from './sessionHistoryProvider';

const DAY = 86_400;
const nowS = () => Math.floor(Date.now() / 1000);
const hrow = (over: Record<string, unknown> = {}) => ({
  session_id: 'a',
  cwd: '/p',
  mode: 'stopped',
  provider: 'codex',
  transport: 'stream',
  updated_at: new Date(Date.now() - 60_000).toISOString(),
  started_at: new Date(Date.now() - 3_600_000).toISOString(),
  last_event_at: nowS() - 60,
  ended_at: nowS() - 60,
  name: 'claudemon-name',
  title: '',
  model: 'gpt-5',
  first_prompt: 'fix the api',
  user_prompts: 3,
  tool_calls: 9,
  transcript_copy: true,
  history: true,
  ...over,
});

beforeEach(() => {
  vi.clearAllMocks();
  resetEvictedOpenRowsForTests();
  callHub.mockResolvedValue(undefined);
  listDaemonSessionRows.mockResolvedValue([]);
  openSessionIds.mockResolvedValue(new Set());
  recent.mockReturnValue([]);
});

describe('historyQuery', () => {
  it('accepts the documented params, clamps limit and ignores unknown keys', () => {
    expect(historyQuery({ cursor: '1:a', limit: 900, query: 'x', cwd: '/p', extra: 1 })).toEqual({
      cursor: '1:a',
      limit: 200,
      query: 'x',
      cwd: '/p',
    });
    expect(historyQuery(undefined)).toEqual({
      cursor: undefined,
      limit: undefined,
      query: undefined,
      cwd: undefined,
    });
  });

  it('refuses malformed params', () => {
    expect(() => historyQuery({ limit: 0 })).toThrow(/limit/);
    expect(() => historyQuery({ limit: 1.5 })).toThrow(/limit/);
    expect(() => historyQuery({ query: 7 })).toThrow(/query/);
    expect(() => historyQuery({ cursor: 'a\nb' })).toThrow(/cursor/);
    expect(() => historyQuery({ cwd: 'x'.repeat(5000) })).toThrow(/cwd/);
    expect(() => historyQuery([1])).toThrow(/object/);
  });
});

describe('historyRows', () => {
  const ctx = (over: Partial<Parameters<typeof historyRows>[1]> = {}) => ({
    hidden: new Set<string>(),
    open: new Set<string>(),
    modes: new Map<string, string>(),
    names: new Map<string, { agentName: string; model: string | null }>(),
    ...over,
  });

  it('projects a stopped history row with the History-only facts', () => {
    const [row] = historyRows([hrow()], ctx({ hidden: new Set(['a']), open: new Set(['a']) }));
    expect(row).toMatchObject({
      sessionId: 'a',
      provider: 'codex',
      cwd: '/p',
      mode: 'stopped',
      transport: 'stream',
      archived: false,
      name: 'claudemon-name',
      model: 'gpt-5',
      hidden: true,
      onSidebar: true,
      firstPrompt: 'fix the api',
      userPrompts: 3,
      toolCalls: 9,
      transcriptCopy: true,
    });
    expect(row.endedAt).toBe((nowS() - 60) * 1000);
  });

  it('merges live mode, prefers the history DB name and drops agent- rows', () => {
    const rows = historyRows(
      [hrow({ session_id: 'live' }), hrow({ session_id: 'agent-x' })],
      ctx({
        modes: new Map([['live', 'responding']]),
        names: new Map([['live', { agentName: 'api-fix', model: 'opus' }]]),
      }),
    );
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ mode: 'responding', name: 'api-fix', model: 'opus' });
    // A live session has no ended time and is never "archived".
    expect(rows[0].endedAt).toBeUndefined();
    expect(rows[0].archived).toBe(false);
  });

  it('marks long-idle stopped rows archived (legacy meaning), separate from hidden', () => {
    const old = new Date(Date.now() - 30 * DAY * 1000).toISOString();
    const [row] = historyRows([hrow({ updated_at: old })], ctx());
    expect(row.archived).toBe(true);
    expect(row.hidden).toBe(false);
  });
});

describe('listSessionHistory', () => {
  it('joins one page with the shared archive, sidebar, live modes and names', async () => {
    fetchDaemonHistory.mockResolvedValue({
      sessions: [hrow({ session_id: 'a', title: '' }), hrow({ session_id: 'b', title: 'Named' })],
      next: '10:b',
    });
    callHub.mockImplementation(async (m: string) =>
      m === 'sessionArchive.get' ? { version: 1, archived: { b: 5 } } : undefined,
    );
    openSessionIds.mockResolvedValue(new Set(['a']));
    listDaemonSessionRows.mockResolvedValue([{ session_id: 'a', mode: 'input' }]);
    recent.mockReturnValue([{ sessionId: 'a', agentName: 'api-fix', model: null }]);
    titleForSession.mockResolvedValue('Fix the API');

    const page = await listSessionHistory({ limit: 2, query: 'fix' });
    expect(fetchDaemonHistory).toHaveBeenCalledWith({
      cursor: undefined,
      limit: 2,
      query: 'fix',
      cwd: undefined,
    });
    expect(page.next).toBe('10:b');
    expect(
      page.sessions.map((s) => [s.sessionId, s.mode, s.hidden, s.onSidebar, s.name, s.title]),
    ).toEqual([
      ['a', 'input', false, true, 'api-fix', 'Fix the API'],
      ['b', 'stopped', true, false, 'claudemon-name', 'Named'],
    ]);
    // The internal transcript path never crosses the wire.
    expect(page.sessions[0]).not.toHaveProperty('transcriptPath');
  });

  it('still answers when the hub cannot be read, and rejects when history cannot', async () => {
    fetchDaemonHistory.mockResolvedValue({ sessions: [hrow()] });
    callHub.mockRejectedValue(new Error('hub down'));
    listDaemonSessionRows.mockResolvedValue(null);
    const page = await listSessionHistory({});
    expect(page).toEqual({
      sessions: [expect.objectContaining({ sessionId: 'a', hidden: false })],
    });

    fetchDaemonHistory.mockRejectedValue(new Error('session history unavailable (500)'));
    await expect(listSessionHistory({})).rejects.toThrow(/unavailable/);
  });
});

describe('forgetSession', () => {
  it('refuses a live session before asking claudemon', async () => {
    getSnapshot.mockReturnValue({ sessionId: 'a', status: 'active' });
    await expect(forgetSession({ sessionId: 'a' })).rejects.toThrow(/live/);
    expect(forgetDaemonSession).not.toHaveBeenCalled();
  });

  it('deletes from history, then drops the id from the shared sidebar and archive', async () => {
    getSnapshot.mockReturnValue({ sessionId: 'a', status: 'ended' });
    forgetDaemonSession.mockResolvedValue({ transcriptCopyRemoved: true });
    await expect(forgetSession({ sessionId: 'a' })).resolves.toEqual({
      ok: true,
      sessionId: 'a',
      transcriptCopyRemoved: true,
    });
    expect(callHub).toHaveBeenCalledWith('sessionOpen.set', { sessionId: 'a', open: false });
    expect(callHub).toHaveBeenCalledWith('sessionArchive.set', { sessionId: 'a', archived: false });
  });

  it('passes claudemon refusals through and never touches the shared view state then', async () => {
    forgetDaemonSession.mockRejectedValue(new Error('session is live; stop it first'));
    await expect(forgetSession({ sessionId: 'a' })).rejects.toThrow(/live/);
    expect(callHub).not.toHaveBeenCalled();
  });

  it('validates the id', async () => {
    for (const sessionId of [undefined, '', 'a/b', 'a b', 'x'.repeat(300), 7]) {
      await expect(forgetSession({ sessionId })).rejects.toThrow(/requires/);
    }
  });
});

describe('fillEvictedOpenRows', () => {
  const stopped = (id: string) => ({ session_id: id, mode: 'stopped', history: true });

  it('reads only sidebar ids nobody holds, and caches found rows', async () => {
    getDaemonSessionRow.mockImplementation(async (id: string) =>
      id === 'gone' ? null : stopped(id),
    );
    const open = new Set(['live', 'old', 'gone']);
    const have = new Set(['live']);
    expect(await fillEvictedOpenRows(open, have, 1_000)).toEqual([stopped('old')]);
    expect(getDaemonSessionRow.mock.calls.map((c) => c[0]).sort()).toEqual(['gone', 'old']);

    getDaemonSessionRow.mockClear();
    expect(await fillEvictedOpenRows(open, have, 2_000)).toEqual([stopped('old')]);
    // A found row is cached; a miss is not retried within a minute.
    expect(getDaemonSessionRow).not.toHaveBeenCalled();
    await fillEvictedOpenRows(open, have, 1_000 + 60_000);
    expect(getDaemonSessionRow.mock.calls.map((c) => c[0])).toEqual(['gone']);
  });

  it('forgets cached rows once the id leaves the sidebar, and bounds reads per call', async () => {
    getDaemonSessionRow.mockImplementation(async (id: string) => stopped(id));
    await fillEvictedOpenRows(new Set(['old']), new Set(), 0);
    getDaemonSessionRow.mockClear();
    await fillEvictedOpenRows(new Set(), new Set(), 0);
    await fillEvictedOpenRows(new Set(['old']), new Set(), 0);
    expect(getDaemonSessionRow).toHaveBeenCalledTimes(1);

    getDaemonSessionRow.mockClear();
    const many = new Set(Array.from({ length: 120 }, (_, i) => `s${i}`));
    const first = await fillEvictedOpenRows(many, new Set(), 0);
    expect(getDaemonSessionRow).toHaveBeenCalledTimes(50);
    expect(first).toHaveLength(50);
    const second = await fillEvictedOpenRows(many, new Set(), 0);
    expect(second).toHaveLength(100);
  });
});
