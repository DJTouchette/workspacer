/**
 * Regression: a boot restore that fails must never be autosaved back.
 *
 * The restore path collapses three different outcomes into the same empty
 * roster — "no saved session", "the file could not be read", and "the file is a
 * shape this build does not understand" — and then the 1s debounced autosave
 * persists whatever is in memory. For the last two that means overwriting the
 * user's layout with `agents: []`, one second after boot, with no backup.
 *
 * The concrete trigger is a nightly→stable rollback: the nightly writes a
 * session shape stable cannot parse, stable boots, renders empty, and erases it.
 * A transient EACCES on the file does the same with no version skew at all.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { useSessionLifecycle } from '../src/hooks/useSessionLifecycle';

let saveSession: ReturnType<typeof vi.fn>;
let listSessions: ReturnType<typeof vi.fn>;
let loadSession: ReturnType<typeof vi.fn>;
let loadAgentsFromSession: ReturnType<typeof vi.fn>;

beforeEach(() => {
  saveSession = vi.fn().mockResolvedValue(undefined);
  listSessions = vi.fn().mockResolvedValue([{ filename: 'default.yaml', name: 'Default' }]);
  loadSession = vi.fn();
  loadAgentsFromSession = vi.fn();
  (window as any).electronAPI = {
    ...(window as any).electronAPI,
    saveSession,
    listSessions,
    loadSession,
    getAllClaudeSessions: vi.fn().mockResolvedValue([]),
    listLiveClaudeSessionIds: vi.fn().mockResolvedValue([]),
    onBeforeQuit: vi.fn().mockReturnValue(() => {}),
  };
});

function render() {
  return renderHook(() =>
    useSessionLifecycle({
      configLoaded: true, // run the startup effect
      agents: [],
      activeAgentId: '',
      loadAgentsFromSession,
      reconcileAgents: vi.fn(),
      appCwdRef: { current: '/x' },
    }),
  );
}

/** Drive the hook to the point where the autosave would fire. */
async function bootThenTrySaving(result: {
  current: { saveCurrentSession: (f?: boolean) => void };
}) {
  await waitFor(() => expect(loadAgentsFromSession).toHaveBeenCalled());
  await act(async () => {
    result.current.saveCurrentSession();
    result.current.saveCurrentSession(true); // even a forced/quit save
  });
}

describe('useSessionLifecycle — a failed restore is never written back', () => {
  it('does not save over a session file this build cannot parse', async () => {
    // A shape from a newer nightly: no `agents`, no `tabs`, no `panes`.
    loadSession.mockResolvedValue({ schemaVersion: 99, roster: [{ id: 'a1' }] });

    const { result } = render();
    await bootThenTrySaving(result);

    expect(
      saveSession,
      'an unrecognised file must be left exactly as it is on disk',
    ).not.toHaveBeenCalled();
  });

  it('does not save over a session file that could not be read', async () => {
    loadSession.mockRejectedValue(new Error('EACCES'));

    const { result } = render();
    await act(async () => {
      await Promise.resolve();
    });
    await act(async () => {
      result.current.saveCurrentSession();
      result.current.saveCurrentSession(true);
    });

    expect(saveSession).not.toHaveBeenCalled();
  });

  it('does not save when listing the sessions throws', async () => {
    listSessions.mockRejectedValue(new Error('EIO'));

    const { result } = render();
    await act(async () => {
      await Promise.resolve();
    });
    await act(async () => {
      result.current.saveCurrentSession();
    });

    expect(saveSession).not.toHaveBeenCalled();
  });

  it('still saves normally after a session restores cleanly', async () => {
    loadSession.mockResolvedValue({ agents: [], activeAgentId: '', name: 'Default' });

    const { result } = render();
    await bootThenTrySaving(result);

    expect(saveSession, 'the guard must not disable saving outright').toHaveBeenCalled();
  });

  it('still saves on a fresh install, where an empty roster is the truth', async () => {
    // No files at all — nothing to overwrite, so saving stays armed.
    listSessions.mockResolvedValue([]);

    const { result } = render();
    await act(async () => {
      await Promise.resolve();
    });
    await act(async () => {
      result.current.saveCurrentSession();
    });

    expect(saveSession).toHaveBeenCalled();
    expect(loadSession, 'nothing to load').not.toHaveBeenCalled();
  });
});
