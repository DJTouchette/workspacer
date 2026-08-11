/**
 * THE QUIT-SAVE HANDSHAKE, when the save FAILS.
 *
 * The handshake exists so main pauses teardown until the roster reached disk
 * (main/index.ts waits on IPC.APP_QUIT_SAVED). Acking unconditionally made a
 * rejected saveSession indistinguishable from a successful one: main quit, the
 * whole agent roster / layout change was lost, and on the next boot the
 * pre-failure file restored and looked completely normal. The same file already
 * has a loud channel for the mirror-image case (a restore that could not be
 * trusted → postNotification 'Session not restored'); nothing used it for a
 * write failure.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { useSessionLifecycle } from '../src/hooks/useSessionLifecycle';
import { postNotification } from '../src/lib/notificationBus';

vi.mock('../src/lib/notificationBus', () => ({ postNotification: vi.fn() }));

let saveSession: ReturnType<typeof vi.fn>;
let notifyQuitSaved: ReturnType<typeof vi.fn>;
let onBeforeQuit: ReturnType<typeof vi.fn>;
let quitCallback: (() => void) | null;

beforeEach(() => {
  vi.mocked(postNotification).mockClear();
  quitCallback = null;
  saveSession = vi
    .fn()
    .mockRejectedValue(
      Object.assign(new Error('ENOSPC: no space left on device'), { code: 'ENOSPC' }),
    );
  notifyQuitSaved = vi.fn();
  onBeforeQuit = vi.fn((cb: () => void) => {
    quitCallback = cb;
    return () => {};
  });
  (window as any).electronAPI = {
    ...(window as any).electronAPI,
    saveSession,
    notifyQuitSaved,
    onBeforeQuit,
    listSessions: vi.fn().mockResolvedValue([]),
    loadSession: vi.fn(),
    getAllClaudeSessions: vi.fn().mockResolvedValue([]),
    listLiveClaudeSessionIds: vi.fn().mockResolvedValue([]),
  };
});

function render() {
  return renderHook(() =>
    useSessionLifecycle({
      configLoaded: true,
      agents: [],
      activeAgentId: '',
      loadAgentsFromSession: vi.fn(),
      reconcileAgents: vi.fn(),
      appCwdRef: { current: '/x' },
    }),
  );
}

describe('useSessionLifecycle — a save that never reached disk', () => {
  it('acks the quit handshake with ok=false instead of a bare "saved"', async () => {
    const { result } = render();
    await waitFor(() => expect(result.current.sessionPhase).toBe('active'));

    await act(async () => {
      quitCallback!();
      await Promise.resolve();
      await Promise.resolve();
    });

    await waitFor(() => expect(notifyQuitSaved).toHaveBeenCalled());
    expect(
      notifyQuitSaved.mock.calls[0]![0],
      'a rejected final save must not ack as if the workspace landed on disk',
    ).toBe(false);
  });

  it('acks true when the save actually landed', async () => {
    saveSession.mockResolvedValue(undefined);
    const { result } = render();
    await waitFor(() => expect(result.current.sessionPhase).toBe('active'));

    await act(async () => {
      quitCallback!();
      await Promise.resolve();
      await Promise.resolve();
    });

    await waitFor(() => expect(notifyQuitSaved).toHaveBeenCalled());
    expect(notifyQuitSaved.mock.calls[0]![0]).toBe(true);
  });

  it('tells the user once that the workspace is not being saved', async () => {
    const { result } = render();
    await waitFor(() => expect(result.current.sessionPhase).toBe('active'));
    await act(async () => {
      await result.current.saveCurrentSession(true);
      await result.current.saveCurrentSession(true);
    });

    const warns = vi
      .mocked(postNotification)
      .mock.calls.filter(([n]) => n.title === 'Workspace not saved');
    expect(warns.length, 'a failed save must not be console.error-only').toBe(1);
    expect(warns[0]![0].level).toBe('warn');
    expect(String(warns[0]![0].body)).toContain('ENOSPC');
  });

  it('resolves false from saveCurrentSession so callers can tell', async () => {
    const { result } = render();
    await waitFor(() => expect(result.current.sessionPhase).toBe('active'));
    let ok: boolean | undefined;
    await act(async () => {
      ok = await result.current.saveCurrentSession(true);
    });
    expect(ok).toBe(false);
  });
});
