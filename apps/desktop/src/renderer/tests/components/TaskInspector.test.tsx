import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { act, render, renderHook, screen, fireEvent } from '@testing-library/react';
import TaskInspector, { selectInspectorTasks } from '../../src/components/TaskInspector';
import { InspectorRail } from '../../src/components/claude/InspectorRail';
import { useAgentManager } from '../../src/hooks/useAgentManager';
import type { DispatchTask } from '../../../../main/shared/dispatchHistory';
import type { ElectronAPI } from '../../src/types/electron';
const makeTask = (id: string, owner = 'manager'): DispatchTask => ({
  taskId: id,
  ownerSessionId: owner,
  ownerLabel: owner,
  title: 'Same title',
  projectCwd: '/project',
  createdAt: '2026-01-01',
  attempts: [],
});
describe('task selection and routing', () => {
  it('uses only exact recorded worker attempts and owner/project facts, with current first', () => {
    const recent = makeTask('recent');
    const active = makeTask('active');
    active.attempts = [
      {
        sessionId: 'worker',
        dispatchId: 'd',
        lifecycle: 'running',
        executionCwd: '/tree',
        worktree: { allocated: true },
      } as never,
    ];
    const foreign = makeTask('foreign', 'other-manager');
    expect(selectInspectorTasks([recent, active, foreign], 'worker').map((t) => t.taskId)).toEqual([
      'active',
    ]);
    expect(selectInspectorTasks([recent, active], 'unknown')).toEqual([]);
    expect(
      selectInspectorTasks([recent, active, foreign], 'manager', true).map((t) => t.taskId),
    ).toEqual(['active', 'recent']);
    expect(selectInspectorTasks([active], 'worker', false, '/tree')).toEqual([active]);
  });
  it('creates and deduplicates real Inspector panes by explicit task target separately from session', () => {
    const { result } = renderHook(() => useAgentManager());
    let first = '';
    act(() => {
      first = result.current.openInspector({ taskId: 'task-1', agentName: 'First task' });
    });
    expect(
      result.current.tabs.flatMap((t) => t.panes).find((p) => p.inspectorTaskId === 'task-1'),
    ).toMatchObject({ type: 'inspector', inspectorTaskId: 'task-1' });
    act(() => {
      expect(result.current.openInspector({ taskId: 'task-1' })).toBe(first);
    });
    act(() => {
      result.current.openInspector({ taskId: 'task-2' });
    });
    act(() => {
      result.current.openInspector({ sessionId: 'worker' });
    });
    expect(
      result.current.tabs.flatMap((t) => t.panes).filter((p) => p.type === 'inspector'),
    ).toHaveLength(3);
  });
  it('mounts Tasks as a fetching rail sibling and leaves the session card alone', async () => {
    const read = vi.fn(async () => ({ available: true, tasks: [makeTask('task')] }));
    window.electronAPI = { dispatchHistoryRead: read } as unknown as ElectronAPI;
    render(<InspectorRail session={null} onClose={() => {}} />);
    expect(read).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'tasks', exact: true }));
    await screen.findByLabelText('Current and recent tasks');
    expect(read).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole('button', { name: 'session', exact: true }));
    expect(screen.queryByLabelText('Task Inspector')).not.toBeInTheDocument();
  });
  it('shows unavailability for missing preload and never reads local history for remote targets', async () => {
    const read = vi.fn();
    window.electronAPI = { dispatchHistoryRead: read } as unknown as ElectronAPI;
    const { unmount } = render(<TaskInspector remote />);
    await screen.findByText(/Task Inspector is available in the local desktop app only/);
    expect(read).not.toHaveBeenCalled();
    unmount();
    window.electronAPI = {} as ElectronAPI;
    render(<TaskInspector />);
    await screen.findByText(/older desktop versions/);
  });
});

it('defaults a recorded manager to its supplied project and can explicitly show all projects', async () => {
  const first = makeTask('first');
  const other = { ...makeTask('other'), projectCwd: '/other-project', title: 'Other project task' };
  window.electronAPI = {
    dispatchHistoryRead: async () => ({
      available: true,
      currentOwnerSessionId: 'manager',
      tasks: [first, other],
    }),
  } as unknown as ElectronAPI;
  render(<TaskInspector sessionId="manager" projectCwd="/project" />);
  await screen.findByLabelText('Task project');
  expect(screen.getByLabelText('Task project')).toHaveValue('/project');
  expect(screen.queryByRole('option', { name: /Other project task/ })).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('Task project'), { target: { value: '' } });
  expect(screen.getByRole('option', { name: /Other project task/ })).toBeInTheDocument();
});
