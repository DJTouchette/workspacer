import React, { useState } from 'react';
import { beforeEach, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentControls, { type IntentControlDraft } from '../../src/components/IntentControls';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';
import type { IntentControl } from '../../../main/shared/intentControl';
const workspace: IntentWorkspace = {
  id: 'work',
  title: 'Feature',
  projectRoot: '/project',
  revision: 2,
  outcome: '',
  constraints: '',
  successCriteria: '',
  sourceUrl: '',
  status: 'active',
  createdAt: '',
  updatedAt: '',
};
const target = {
  sessionId: 'worker',
  hub: '',
  label: 'Worker',
  provider: 'codex',
  cwd: '/project',
};
const record: IntentControl = {
  id: 'control',
  workspaceId: 'work',
  executionId: 'run',
  intentRevision: 2,
  target,
  kind: 'interrupt',
  author: 'user',
  text: 'Review first',
  packet: 'Exact saved control',
  createdAt: '2026-09-12T12:00:00Z',
  attempts: [],
  reconciliations: [],
};
const request = vi.fn();
let records: IntentControl[];
beforeEach(() => {
  records = [];
  request.mockReset().mockImplementation(async (input) => {
    if (input.action === 'controls')
      return { action: 'controls', controls: structuredClone(records) };
    if (input.action === 'executions')
      return { action: 'executions', executions: [{ id: 'run', session: target }], links: [] };
    if (input.action === 'prepareControl') {
      records = [{ ...record, id: input.controlId, kind: input.kind, text: input.text }];
      return { action: 'prepareControl', control: records[0], created: true };
    }
    if (input.action === 'sendControl') {
      records[0].attempts = [
        {
          id: input.attemptId,
          status: 'accepted',
          startedAt: record.createdAt,
          detail: 'Service accepted request',
        },
      ];
      return { action: 'sendControl', control: records[0], dispatched: true };
    }
    if (input.action === 'reconcileControl') {
      records[0].reconciliations.push({
        id: input.reconciliationId,
        author: 'user',
        assessment: input.assessment,
        reason: input.reason,
        at: record.createdAt,
      });
      return { action: 'reconcileControl', control: records[0] };
    }
    throw new Error('Unexpected action');
  });
  Object.assign(window.electronAPI, {
    intentWorkspaceRequest: request,
    claudeSignal: vi.fn(),
    claudeMessage: vi.fn(),
  });
});
function View({ visible = true, disabled = false } = {}) {
  const [draft, setDraft] = useState<IntentControlDraft>({
    executionId: '',
    kind: 'interrupt',
    text: '',
  });
  return (
    <IntentControls
      workspace={workspace}
      visible={visible}
      disabled={disabled}
      draft={draft}
      onDraftChange={setDraft}
      sessions={[target]}
    />
  );
}
async function fill() {
  await screen.findByRole('option', { name: /Worker/ });
  fireEvent.change(screen.getByLabelText('Control target'), { target: { value: 'run' } });
  fireEvent.change(screen.getByLabelText('Reason for interrupting (recorded here)'), {
    target: { value: 'Review first' },
  });
}
it('records a control for review and sends only the persisted identity after a separate click', async () => {
  render(<View />);
  await fill();
  fireEvent.click(screen.getByRole('button', { name: 'Save control for review' }));
  expect(await screen.findByText('Exact saved control')).toBeInTheDocument();
  expect(request.mock.calls.some(([input]) => input.action === 'sendControl')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'Send interrupt request' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith({
      action: 'sendControl',
      id: 'work',
      controlId: records[0].id,
      attemptId: expect.any(String),
    }),
  );
  expect(await screen.findByText(/Service acceptance does not establish/)).toBeInTheDocument();
  expect(window.electronAPI.claudeSignal).not.toHaveBeenCalled();
  expect(window.electronAPI.claudeMessage).not.toHaveBeenCalled();
});
it('retains failed drafts and claim identity across view navigation', async () => {
  const { rerender } = render(<View />);
  await fill();
  const normal = request.getMockImplementation()!;
  request.mockImplementation(async (input) => {
    if (input.action === 'prepareControl') throw new Error('Disk unavailable');
    return normal(input);
  });
  fireEvent.click(screen.getByRole('button', { name: 'Save control for review' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Disk unavailable');
  const first = request.mock.calls.find(([input]) => input.action === 'prepareControl')![0];
  rerender(<View visible={false} />);
  rerender(<View />);
  expect(screen.getByLabelText('Reason for interrupting (recorded here)')).toHaveValue(
    'Review first',
  );
  fireEvent.click(screen.getByRole('button', { name: 'Save control for review' }));
  await waitFor(() =>
    expect(request.mock.calls.filter(([input]) => input.action === 'prepareControl')).toHaveLength(
      2,
    ),
  );
  expect(request.mock.calls.filter(([input]) => input.action === 'prepareControl')[1][0]).toEqual(
    first,
  );
});
it('reads durable uncertainty after lost responses and records a user assessment without exposing retry', async () => {
  records = [structuredClone(record)];
  const normal = request.getMockImplementation()!;
  request.mockImplementation(async (input) => {
    if (input.action === 'sendControl') {
      records[0].attempts = [
        {
          id: input.attemptId,
          status: 'unknown',
          startedAt: record.createdAt,
          detail: 'Acknowledgment lost',
        },
      ];
      throw new Error('Connection closed');
    }
    return normal(input);
  });
  render(<View />);
  fireEvent.click(await screen.findByRole('button', { name: 'Send interrupt request' }));
  expect(await screen.findByText('Acknowledgment lost')).toBeInTheDocument();
  fireEvent.click(screen.getByText('Your assessment'));
  fireEvent.change(screen.getByLabelText('Assessment'), { target: { value: 'observed' } });
  fireEvent.change(screen.getByLabelText('What did you observe?'), {
    target: { value: 'Transcript shows an interrupted turn' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Record assessment' }));
  expect(await screen.findByText('Transcript shows an interrupted turn')).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Retry control delivery' })).not.toBeInTheDocument();
  expect(request.mock.calls.filter(([input]) => input.action === 'sendControl')).toHaveLength(1);
  expect(records[0].attempts[0].status).toBe('unknown');
});
it('disables earlier revision sends and unavailable qualified targets', async () => {
  records = [{ ...record, intentRevision: 1 }];
  const { unmount } = render(<View />);
  expect(await screen.findByText(/Record a new control using/)).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Send interrupt request' })).not.toBeInTheDocument();
  unmount();
  records = [{ ...record, target: { ...target, hub: 'peer' } }];
  render(<View />);
  expect(await screen.findByRole('button', { name: 'Send interrupt request' })).toBeDisabled();
});

it('retains assessment drafts across tab changes and refuses an unreviewed concurrent assessment', async () => {
  records = [
    {
      ...record,
      attempts: [{ id: 'a', status: 'accepted', detail: 'Accepted', startedAt: record.createdAt }],
    },
  ];
  const rendered = render(<View />);
  fireEvent.click(await screen.findByText('Your assessment'));
  fireEvent.change(screen.getByLabelText('What did you observe?'), {
    target: { value: 'My observation in progress' },
  });
  rendered.rerender(<View visible={false} />);
  rendered.rerender(<View />);
  await screen.findByText('Your assessment');
  fireEvent.click(screen.getByText('Your assessment'));
  expect(screen.getByLabelText('What did you observe?')).toHaveValue('My observation in progress');
  records[0].reconciliations = [
    {
      id: 'remote-assessment',
      assessment: 'observed',
      reason: 'Other client observation',
      author: 'user',
      at: record.createdAt,
    },
  ];
  fireEvent.click(screen.getByRole('button', { name: 'Refresh controls' }));
  await screen.findByText('Other client observation');
  expect(screen.getByRole('button', { name: 'Record assessment' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: 'I reviewed the latest assessments' }));
  fireEvent.click(screen.getByRole('button', { name: 'Record assessment' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'reconcileControl',
        expectedCount: 1,
        reason: 'My observation in progress',
      }),
    ),
  );
});

it('does not apply a delayed control mutation to another workspace', async () => {
  const normal = request.getMockImplementation()!;
  let resolve!: (value: unknown) => void;
  request.mockImplementation((input) =>
    input.action === 'prepareControl'
      ? new Promise((r) => {
          resolve = r;
        })
      : normal(input),
  );
  const change = vi.fn();
  const draft = { executionId: 'run', kind: 'interrupt' as const, text: 'Old workspace control' };
  const rendered = render(
    <IntentControls
      workspace={workspace}
      visible
      disabled={false}
      draft={draft}
      onDraftChange={change}
      sessions={[target]}
    />,
  );
  await screen.findByRole('option', { name: /Worker/ });
  fireEvent.click(screen.getByRole('button', { name: 'Save control for review' }));
  await waitFor(() => expect(resolve).toBeTypeOf('function'));
  const other = { ...workspace, id: 'other' };
  rendered.rerender(
    <IntentControls
      workspace={other}
      visible
      disabled={false}
      draft={{ ...draft, text: 'New workspace draft' }}
      onDraftChange={change}
      sessions={[target]}
    />,
  );
  await waitFor(() => expect(request).toHaveBeenCalledWith({ action: 'controls', id: 'other' }));
  const calls = change.mock.calls.length;
  await act(async () => resolve({ action: 'prepareControl', control: record, created: true }));
  expect(change).toHaveBeenCalledTimes(calls);
  expect(screen.getByLabelText('Reason for interrupting (recorded here)')).toHaveValue(
    'New workspace draft',
  );
  expect(
    request.mock.calls.filter(([r]) => r.action === 'controls' && r.id === 'work'),
  ).toHaveLength(1);
});
