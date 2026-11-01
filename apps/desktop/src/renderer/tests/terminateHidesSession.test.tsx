/**
 * Closing an agent card HIDES its session (docs/session-roster.md: Hide =
 * archive): `sessionArchive.set {archived:true}` for its live and resumable
 * ids, the same Hide every client writes, so the hub drops it from the shared
 * sidebar. Best-effort: an older hub without the archive, or a failing call,
 * never blocks the close; a federated card's session belongs to its peer hub
 * and is left alone.
 */
import { describe, it, expect, vi, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useAgentManager } from '../src/hooks/useAgentManager';

const seedAgent = (id: string, extra: Record<string, unknown> = {}) =>
  ({
    id,
    name: id,
    cwd: '/repo',
    tabs: [{ id: `${id}-t`, title: id, panes: [], activePaneId: '' }],
    activeTabId: `${id}-t`,
    ...extra,
  }) as any;

const api = window.electronAPI as any;
const original = api.sessionArchiveSet;

describe('terminateAgent hides the session (sessionArchive.set true)', () => {
  afterEach(() => {
    api.sessionArchiveSet = original;
  });

  it('archives the live and resumable ids once each', async () => {
    api.sessionArchiveSet = vi.fn().mockResolvedValue({ version: 1, archived: {} });
    const hook = renderHook(() => useAgentManager());
    act(() =>
      hook.result.current.loadAgentsFromSession(
        [
          seedAgent('a', { sessionId: 'sa', lastSessionId: 'old' }),
          seedAgent('b', { sessionId: 'sb', lastSessionId: 'sb' }),
        ],
        'a',
      ),
    );
    await act(async () => hook.result.current.terminateAgent('a'));
    expect(api.sessionArchiveSet.mock.calls).toEqual([
      ['sa', true],
      ['old', true],
    ]);
    await act(async () => hook.result.current.terminateAgent('b'));
    expect(api.sessionArchiveSet.mock.calls.slice(2)).toEqual([['sb', true]]);
    hook.unmount();
  });

  it('closes anyway when the hub refuses or lacks the archive', async () => {
    api.sessionArchiveSet = vi.fn().mockRejectedValue(new Error('no provider'));
    const hook = renderHook(() => useAgentManager());
    act(() =>
      hook.result.current.loadAgentsFromSession(
        [seedAgent('a', { sessionId: 'sa' }), seedAgent('b', { sessionId: 'sb' })],
        'a',
      ),
    );
    const ids = () =>
      hook.result.current.agents.filter((x: any) => !x.global).map((x: any) => x.id);
    await act(async () => hook.result.current.terminateAgent('a'));
    expect(ids()).toEqual(['b']);

    api.sessionArchiveSet = undefined;
    await act(async () => hook.result.current.terminateAgent('b'));
    expect(ids()).toEqual([]);
    hook.unmount();
  });

  it("never archives a federated card's session locally", async () => {
    api.sessionArchiveSet = vi.fn().mockResolvedValue({ version: 1, archived: {} });
    api.claudeSignal = api.claudeSignal ?? vi.fn().mockResolvedValue(undefined);
    const hook = renderHook(() => useAgentManager());
    act(() =>
      hook.result.current.loadAgentsFromSession(
        [seedAgent('r', { sessionId: 'remote', hub: 'peer' })],
        'r',
      ),
    );
    await act(async () => hook.result.current.terminateAgent('r'));
    expect(api.sessionArchiveSet).not.toHaveBeenCalled();
    hook.unmount();
  });
});
