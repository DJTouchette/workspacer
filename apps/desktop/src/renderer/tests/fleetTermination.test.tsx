import { useSessionChatState } from '../src/hooks/useSessionChatUiState';
import React from 'react';
import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { TerminateAgentButton } from '../src/components/TerminateAgentButton';
import { findAgentChatPane, useAgentManager } from '../src/hooks/useAgentManager';
import { resetTerminatedSessions, wasSessionTerminated } from '../src/lib/terminatedSessions';
import { promoteSessionSnapshots } from '../src/lib/promoteSessionSnapshots';
const agent = (hub?: string) =>
  ({
    id: 'a',
    name: 'Agent',
    hub,
    cwd: '/fixture',
    sessionId: 's-a',
    tabs: [
      {
        id: 't',
        title: 'Agent',
        activePaneId: 'p',
        panes: [{ id: 'p', type: 'claude', title: 'Agent', attachSessionId: 's-a' }],
      },
    ],
    activeTabId: 't',
  }) as any;
beforeEach(() => resetTerminatedSessions());
describe('Fleet termination control', () => {
  it('confirms, cancels, preserves errors, and never activates the card', async () => {
    const activate = vi.fn();
    const terminate = vi.fn().mockRejectedValue(new Error('Authority denied'));
    render(
      <div onClick={activate} onMouseDown={activate} onKeyDown={activate}>
        <TerminateAgentButton agent={agent()} onTerminate={terminate} />
      </div>,
    );
    fireEvent.mouseDown(screen.getByRole('button', { name: /Terminate/ }));
    fireEvent.click(screen.getByRole('button', { name: /Terminate/ }));
    expect(terminate).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(terminate).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: /Terminate/ }));
    fireEvent.keyDown(screen.getByRole('button', { name: 'Confirm terminate' }), { key: 'Enter' });
    fireEvent.click(screen.getByRole('button', { name: 'Confirm terminate' }));
    await screen.findByRole('alert');
    expect(screen.getByRole('alert')).toHaveTextContent('Authority denied');
    expect(activate).not.toHaveBeenCalled();
    expect(terminate).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeEnabled();
  });
  it('submits once and disables both controls while waiting', async () => {
    let resolve!: () => void;
    const terminate = vi.fn(
      () =>
        new Promise<void>((r) => {
          resolve = r;
        }),
    );
    render(<TerminateAgentButton agent={agent()} onTerminate={terminate} />);
    fireEvent.click(screen.getByRole('button', { name: /Terminate/ }));
    const confirm = screen.getByRole('button', { name: 'Confirm terminate' });
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    expect(terminate).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
    await act(async () => resolve());
    expect(screen.queryByText('Confirm terminate')).not.toBeInTheDocument();
  });
  it('explains offline and unavailable remote control without sending', () => {
    const terminate = vi.fn();
    const view = render(
      <TerminateAgentButton
        agent={agent('peer')}
        snapshot={{ hubOffline: true } as any}
        onTerminate={terminate}
      />,
    );
    expect(screen.getByRole('button')).toBeDisabled();
    expect(screen.getByText(/Hub is offline/)).toBeInTheDocument();
    view.rerender(<TerminateAgentButton agent={agent('peer')} onTerminate={terminate} />);
    expect(screen.getByText(/authority is unavailable/)).toBeInTheDocument();
    expect(screen.getByRole('button')).toBeDisabled();
    expect(terminate).not.toHaveBeenCalled();
  });
});
describe('canonical local and remote termination', () => {
  it.each([undefined, 'peer'])(
    'preserves the agent until %s authority succeeds, then tombstones late snapshots',
    async (hub) => {
      let resolve!: () => void;
      const request = vi.fn(
        () =>
          new Promise<void>((r) => {
            resolve = r;
          }),
      );
      window.electronAPI.claudeClose = request;
      window.electronAPI.claudeSignal = request;
      const hook = renderHook(() => useAgentManager());
      act(() => hook.result.current.loadAgentsFromSession([agent(hub)], 'a'));
      let pending!: Promise<void>;
      act(() => {
        pending = hook.result.current.terminateAgent('a');
        expect(hook.result.current.terminateAgent('a')).toBe(pending);
      });
      expect(hook.result.current.agents.some((a) => a.id === 'a')).toBe(true);
      expect(wasSessionTerminated('s-a')).toBe(false);
      expect(request).toHaveBeenCalledTimes(1);
      expect(request).toHaveBeenCalledWith(...(hub ? ['s-a', 'SIGTERM'] : ['s-a']));
      await act(async () => {
        resolve();
        await pending;
      });
      expect(hook.result.current.agents.some((a) => a.id === 'a')).toBe(false);
      expect(wasSessionTerminated('s-a')).toBe(true);
      expect(
        promoteSessionSnapshots([
          { sessionId: 's-a', status: 'active', ambientState: 'streaming' } as any,
        ]).snapshotBySession,
      ).toEqual({});
    },
  );
  it.each([undefined, 'peer'])('retains the card and pin on rejected %s request', async (hub) => {
    const reject = vi.fn().mockRejectedValue(new Error('Authority denied'));
    window.electronAPI.claudeClose = reject;
    window.electronAPI.claudeSignal = reject;
    const closed = vi.fn();
    window.addEventListener('agent:closed', closed);
    const hook = renderHook(() => useAgentManager());
    act(() => hook.result.current.loadAgentsFromSession([agent(hub)], 'a'));
    await act(async () => {
      await expect(hook.result.current.terminateAgent('a')).rejects.toThrow('Authority denied');
    });
    expect(hook.result.current.agents.some((a) => a.id === 'a')).toBe(true);
    expect(wasSessionTerminated('s-a')).toBe(false);
    expect(closed).not.toHaveBeenCalled();
    window.removeEventListener('agent:closed', closed);
  });
});

it('dismisses a stopped card without a process request and releases retained session state', async () => {
  window.electronAPI.claudeClose = vi.fn();
  window.electronAPI.claudeSignal = vi.fn();
  const retained = renderHook(() => useSessionChatState('stopped-session', 'draft', 'old draft'));
  const hook = renderHook(() => useAgentManager());
  act(() =>
    hook.result.current.loadAgentsFromSession(
      [{ ...agent(), sessionId: undefined, lastSessionId: 'stopped-session' }],
      'a',
    ),
  );
  await act(async () => hook.result.current.terminateAgent('a'));
  expect(hook.result.current.agents.some((a) => a.id === 'a')).toBe(false);
  expect(window.electronAPI.claudeClose).not.toHaveBeenCalled();
  expect(window.electronAPI.claudeSignal).not.toHaveBeenCalled();
  retained.unmount();
  const fresh = renderHook(() => useSessionChatState('stopped-session', 'draft', ''));
  expect(fresh.result.current[0]).toBe('');
});

it('restores only a missing owning chat and keeps the current workspace selection', () => {
  const hook = renderHook(() => useAgentManager());
  const worker = {
    ...agent(),
    tabs: [
      {
        id: 't',
        title: 'Tools',
        activePaneId: 'watch',
        panes: [
          {
            id: 'watch',
            title: 'Another session',
            type: 'claude',
            attachSessionId: 'different-session',
          },
        ],
      },
    ],
  };
  act(() =>
    hook.result.current.loadAgentsFromSession(
      [worker, { ...agent(), id: 'manager', sessionId: 'manager-session' }],
      'manager',
    ),
  );
  act(() => {
    hook.result.current.ensureAgentChat('a');
    hook.result.current.ensureAgentChat('a');
  });
  const restored = hook.result.current.agents.find((a) => a.id === 'a')!;
  expect(
    restored.tabs.flatMap((t) => t.panes).filter((p) => p.attachSessionId === 's-a'),
  ).toHaveLength(1);
  expect(findAgentChatPane(restored)?.attachSessionId).toBe('s-a');
  expect(hook.result.current.activeAgentId).toBe('manager');
});
