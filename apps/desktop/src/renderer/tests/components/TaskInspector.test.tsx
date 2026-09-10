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

describe('compact default view', () => {
  const workflowTask = (): DispatchTask => ({
    ...makeTask('task-1'),
    title: 'Ship the inspector',
    revision: 3,
    workflow: {
      hash: 'pin-hash',
      definition: {
        id: 'w',
        revision: 2,
        name: 'Scout implement review',
        description: '',
        enabled: true,
        steps: [
          {
            id: 'implement',
            label: 'Implement',
            kind: 'implement',
            stage: 'implement',
            role: 'implementer',
            template: 't',
            instructions: 'Do the work',
          },
          {
            id: 'review',
            label: 'Review',
            kind: 'review',
            stage: 'review',
            role: 'reviewer',
            template: 't',
            instructions: 'Review the work',
          },
        ],
      },
      templates: { t: { id: 't', body: 'body', params: [], resultSchema: {} } },
      steps: [
        { id: 'implement', state: 'completed', reason: 'A long completed rationale' },
        { id: 'review', state: 'planned' },
      ],
    } as never,
  });
  const mount = async (task: DispatchTask) => {
    window.electronAPI = {
      dispatchHistoryRead: async () => ({
        available: true,
        currentOwnerSessionId: 'manager',
        tasks: [task],
      }),
    } as unknown as ElectronAPI;
    render(<TaskInspector sessionId="manager" projectCwd="/project" />);
    await screen.findByLabelText('Current and recent tasks');
  };
  it('shows the title once and keeps ids, paths and finished rationale out of the default view', async () => {
    await mount(workflowTask());
    // The <option> in the switcher carries the title too; nothing else may.
    const titled = screen
      .getAllByText('Ship the inspector')
      .filter((el) => el.tagName !== 'OPTION');
    expect(titled).toHaveLength(1);
    expect(screen.getByLabelText('Current and recent tasks')).not.toBeVisible();
    // Present in the DOM but behind a closed disclosure — preserved, not shown.
    expect(screen.getByText('task-1')).not.toBeVisible();
    expect(screen.getByText('/project')).not.toBeVisible();
    expect(screen.getByText(/pin-hash/)).not.toBeVisible();
    // A completed step's rationale is collapsed, not deleted.
    expect(screen.getAllByText('A long completed rationale')[0]).not.toBeVisible();
  });
  it('offers a skip only on unfinished steps and never a disabled one on a finished step', async () => {
    await mount(workflowTask());
    expect(screen.queryByRole('button', { name: 'Skip Implement…' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Skip Review…' })).toBeEnabled();
  });
  it('hides a running step action and keeps its explanation under Step details', async () => {
    const task = workflowTask();
    task.workflow!.steps[0].state = 'dispatched';
    task.workflow!.steps[0].reason = undefined;
    task.ownerLabel = task.ownerSessionId;
    await mount(task);
    expect(screen.queryByRole('button', { name: 'Skip Implement…' })).not.toBeInTheDocument();
    expect(screen.getByText('A worker has been dispatched for this step')).not.toBeVisible();
    expect(screen.getByText(task.ownerSessionId, { exact: true })).not.toBeVisible();
  });
  it('exposes ids behind Details with a copy affordance', async () => {
    await mount(workflowTask());
    fireEvent.click(screen.getByText('Details'));
    expect(screen.getByText('task-1')).toBeInTheDocument();
    expect(screen.getByLabelText('Copy task')).toBeInTheDocument();
    expect(screen.getByLabelText('Copy project')).toBeInTheDocument();
  });
  it('renders references as chips and hides the editor until asked', async () => {
    const task = workflowTask();
    task.links = {
      pullRequest: { number: '9492', url: 'https://dev.azure.test/pullrequest/9492' },
      tickets: [{ id: 'WKS-1' }],
    };
    await mount(task);
    expect(screen.getByRole('button', { name: /PR 9492/ })).toBeInTheDocument();
    expect(screen.getByText('WKS-1')).toBeInTheDocument();
    expect(screen.queryByLabelText('PR number')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Links/ }));
    expect(screen.getByLabelText('PR number')).toHaveValue('9492');
  });
  it('shows only recorded link actors and timestamps under Details, with unknown older origins', async () => {
    const task = workflowTask();
    task.audit = [
      {
        id: 'm',
        action: 'links',
        actor: 'manager',
        reason: 'Updated references',
        createdAt: '2026-09-09T03:00:00Z',
      },
      {
        id: 'h',
        action: 'links',
        actor: 'host-user',
        reason: 'Updated references',
        createdAt: '2026-09-09T04:00:00Z',
      },
      {
        id: 'w',
        action: 'waive',
        actor: 'host-user',
        reason: 'Not a links edit',
        createdAt: '2026-09-09T05:00:00Z',
      },
    ];
    await mount(task);
    const history = screen.getByLabelText('Reference edit history');
    expect(history).not.toBeVisible();
    fireEvent.click(screen.getByText('Details', { exact: true }));
    expect(history).toBeVisible();
    expect(history).toHaveTextContent('References updated by manager');
    expect(history).toHaveTextContent('References updated by you');
    expect(Array.from(history.querySelectorAll('time'), (t) => t.dateTime)).toEqual([
      '2026-09-09T03:00:00Z',
      '2026-09-09T04:00:00Z',
    ]);
    expect(history).not.toHaveTextContent('Not a links edit');
    expect(history).toHaveTextContent(
      'Individual link origins and older edits outside this recorded history are unknown.',
    );
  });
  it('does not attribute legacy references with no recorded audit to a manager or host user', async () => {
    const task = workflowTask();
    task.links = { tickets: [{ id: 'LEGACY' }] };
    await mount(task);
    const history = screen.getByLabelText('Reference edit history');
    expect(history).not.toHaveTextContent('References updated by');
    expect(history).toHaveTextContent('unknown');
    expect(screen.getByText('LEGACY').closest('[title]')).toHaveAttribute(
      'title',
      'Recorded reference; edit history in Details. Not verified with the provider.',
    );
  });
});

it('shows a linked task before dispatch and selects its concrete dependency without exposing IDs', async () => {
  const fixes = { ...makeTask('fix-id'), title: 'Current fixes' };
  const nightly = {
    ...makeTask('nightly-id'),
    title: 'Publish nightly',
    dependsOn: ['fix-id'],
    sources: [{ requestId: 'private-request-id', intentKey: 'nightly', label: 'Request today' }],
  };
  window.electronAPI = {
    dispatchHistoryRead: async () => ({
      available: true,
      currentOwnerSessionId: 'manager',
      tasks: [nightly, fixes],
      requests: [
        {
          requestId: 'unresolved',
          ownerSessionId: 'manager',
          resolved: false,
          delivery: 'unknown',
        },
      ],
    }),
  } as unknown as ElectronAPI;
  render(<TaskInspector taskId="nightly-id" sessionId="manager" manager />);
  await screen.findByRole('heading', { name: 'Publish nightly' });
  expect(screen.getByText('Waiting for accepted evidence')).toBeVisible();
  expect(screen.getByText('Source: Request today')).toBeVisible();
  expect(screen.getByText(/Some chat deliveries are unknown/)).toBeVisible();
  expect(screen.queryByText('private-request-id')).not.toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: 'Current fixes' }));
  await screen.findByRole('heading', { name: 'Current fixes' });
});

it('shows the PR chip immediately from a persisted production resolver result', async () => {
  const fs = await import('node:fs');
  const os = await import('node:os');
  const path = await import('node:path');
  const { ManagerRequestService } = await import('../../../main/services/managerRequestService');
  const { DispatchHistoryStore } = await import('../../../main/services/dispatchHistoryStore');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'inspector-resolver-'));
  const store = new DispatchHistoryStore(() => path.join(dir, 'history.json'));
  const service = new ManagerRequestService(
    store,
    (id) => ({ sessionId: id, cwd: dir, isWakeTarget: true, status: 'active' }),
    () => ({
      hash: 'fixture',
      templates: {},
      definition: {
        id: 'fixture',
        name: 'Fixture',
        revision: 1,
        enabled: true,
        description: '',
        steps: [],
      },
      steps: [],
    }),
  );
  let view: ReturnType<typeof render> | undefined;
  try {
    const request = service.prepare(
      'manager',
      'Fix https://business.visualstudio.com/Project/_git/Repo/pullrequest/9492',
    );
    if (!request.available) throw new Error('Capture unavailable');
    const delivery = service.beginDelivery('manager', request.requestId)!;
    service.finishDelivery(request.requestId, delivery.deliveryId, 'accepted');
    expect(
      service.handle(
        {
          op: 'resolveRequest',
          requestId: request.requestId,
          expectedRevision: service.request('manager', request.requestId).revision,
          intents: [
            {
              key: 'fix',
              kind: 'create',
              title: 'Fix PR',
              cwd: dir,
              provenance: 'explicit',
              reason: 'Original user request',
            },
          ],
        },
        'manager',
      ).ok,
    ).toBe(true);
    window.electronAPI = {
      dispatchHistoryRead: async () => ({
        available: true,
        currentOwnerSessionId: 'manager',
        tasks: store.list(),
      }),
    } as unknown as ElectronAPI;
    view = render(<TaskInspector />);
    expect(await screen.findByText('PR 9492')).toBeVisible();
  } finally {
    view?.unmount();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
