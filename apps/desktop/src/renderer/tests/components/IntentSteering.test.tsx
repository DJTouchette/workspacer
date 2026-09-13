import React, { useState } from 'react';
import { beforeEach, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentSteering, { type IntentDirectionDraft } from '../../src/components/IntentSteering';
import type { IntentDirection, IntentWorkspace } from '../../../main/shared/intentWorkspace';

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
const record: IntentDirection = {
  id: 'direction',
  workspaceId: 'work',
  executionId: 'run',
  intentRevision: 2,
  target,
  author: 'user',
  text: 'Preserve permissions',
  packet: 'Exact saved message',
  createdAt: '2026-09-12T12:00:00Z',
  attempts: [],
};
const request = vi.fn();
let records: IntentDirection[];
beforeEach(() => {
  records = [];
  request.mockReset().mockImplementation(async (input) => {
    if (input.action === 'directions')
      return { action: 'directions', directions: structuredClone(records) };
    if (input.action === 'executions')
      return { action: 'executions', executions: [{ id: 'run', session: target }], links: [] };
    if (input.action === 'prepareDirection') {
      records = [{ ...record, id: input.directionId, text: input.text }];
      return { action: 'prepareDirection', created: true, direction: records[0] };
    }
    if (input.action === 'sendDirection') {
      records[0].attempts = [
        {
          id: input.attemptId,
          status: 'accepted',
          startedAt: '2026-09-12T12:00:00Z',
          detail: 'Transport accepted',
        },
      ];
      return { action: 'sendDirection', dispatched: true, direction: records[0] };
    }
    throw new Error('Unexpected request');
  });
  Object.assign(window.electronAPI, { intentWorkspaceRequest: request, claudeMessage: vi.fn() });
});
function View({ visible = true, disabled = false } = {}) {
  const [draft, setDraft] = useState<IntentDirectionDraft>({ executionId: '', text: '' });
  return (
    <IntentSteering
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
  fireEvent.change(screen.getByLabelText('Target execution'), { target: { value: 'run' } });
  fireEvent.change(screen.getByLabelText('What should change?'), {
    target: { value: 'Preserve permissions' },
  });
}
it('saves for review without sending and sends only the persisted direction identity', async () => {
  render(<View />);
  await fill();
  fireEvent.click(screen.getByRole('button', { name: 'Save direction for review' }));
  expect(await screen.findByText('Exact saved message')).toBeInTheDocument();
  expect(request.mock.calls.some(([input]) => input.action === 'sendDirection')).toBe(false);
  expect(window.electronAPI.claudeMessage).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Send direction' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith({
      action: 'sendDirection',
      id: 'work',
      directionId: records[0].id,
      attemptId: expect.any(String),
    }),
  );
  expect(await screen.findByText(/does not confirm that the agent understood/)).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Send direction' })).not.toBeInTheDocument();
});
it('retains a failed draft and its id while navigating away and back', async () => {
  const { rerender } = render(<View />);
  await fill();
  const normal = request.getMockImplementation()!;
  request.mockImplementation(async (input) => {
    if (input.action === 'prepareDirection') throw new Error('Save unavailable');
    return normal(input);
  });
  fireEvent.click(screen.getByRole('button', { name: 'Save direction for review' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Save unavailable');
  const first = request.mock.calls.find(([input]) => input.action === 'prepareDirection')![0];
  rerender(<View visible={false} />);
  rerender(<View />);
  expect(screen.getByLabelText('What should change?')).toHaveValue('Preserve permissions');
  fireEvent.click(screen.getByRole('button', { name: 'Save direction for review' }));
  await waitFor(() =>
    expect(
      request.mock.calls.filter(([input]) => input.action === 'prepareDirection'),
    ).toHaveLength(2),
  );
  expect(request.mock.calls.filter(([input]) => input.action === 'prepareDirection')[1][0]).toEqual(
    first,
  );
});
it('reads the durable uncertain receipt after a lost send response without replaying', async () => {
  records = [structuredClone(record)];
  const normal = request.getMockImplementation()!;
  request.mockImplementation(async (input) => {
    if (input.action === 'sendDirection') {
      records[0].attempts = [
        { id: input.attemptId, status: 'unknown', startedAt: '', detail: 'Acknowledgment lost' },
      ];
      throw new Error('Connection closed');
    }
    return normal(input);
  });
  render(<View />);
  fireEvent.click(await screen.findByRole('button', { name: 'Send direction' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Connection closed');
  expect(await screen.findByText('Acknowledgment lost')).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Retry delivery' })).not.toBeInTheDocument();
  expect(request.mock.calls.filter(([input]) => input.action === 'sendDirection')).toHaveLength(1);
});
it('prepares replacements on the same execution and disables stale or unsaved sends', async () => {
  records = [{ ...record, intentRevision: 1 }];
  const { rerender } = render(<View />);
  fireEvent.click(await screen.findByRole('button', { name: 'Replace direction' }));
  expect(screen.getByLabelText('Target execution')).toBeDisabled();
  expect(screen.getByLabelText('What should change?')).toHaveValue(record.text);
  expect(screen.queryByRole('button', { name: 'Send direction' })).not.toBeInTheDocument();
  rerender(<View disabled />);
  expect(screen.getByRole('button', { name: 'Save direction for review' })).toBeDisabled();
});

it('keeps assessment text when leaving and returning to Direction', async () => {
  records = [
    {
      ...record,
      attempts: [
        { id: 'attempt', status: 'accepted', detail: 'Accepted', startedAt: record.createdAt },
      ],
    },
  ];
  const rendered = render(<View />);
  fireEvent.click(await screen.findByText('Your assessment'));
  fireEvent.change(screen.getByLabelText('What did you observe?'), {
    target: { value: 'Checking the result carefully' },
  });
  rendered.rerender(<View visible={false} />);
  rendered.rerender(<View />);
  fireEvent.click(await screen.findByText('Your assessment'));
  expect(screen.getByLabelText('What did you observe?')).toHaveValue(
    'Checking the result carefully',
  );
});

it('does not clear a different workspace draft after a previous workspace save completes', async () => {
  const normal = request.getMockImplementation()!;
  let resolve!: (value: unknown) => void;
  request.mockImplementation((input) =>
    input.action === 'prepareDirection'
      ? new Promise((r) => {
          resolve = r;
        })
      : normal(input),
  );
  const onDraftChange = vi.fn();
  const draft = { executionId: 'run', text: 'Previous workspace request' };
  const rendered = render(
    <IntentSteering
      workspace={workspace}
      visible
      disabled={false}
      draft={draft}
      onDraftChange={onDraftChange}
      sessions={[target]}
    />,
  );
  await screen.findByRole('option', { name: /Worker/ });
  fireEvent.click(screen.getByRole('button', { name: 'Save direction for review' }));
  await waitFor(() => expect(resolve).toBeTypeOf('function'));
  rendered.rerender(
    <IntentSteering
      workspace={{ ...workspace, id: 'other' }}
      visible
      disabled={false}
      draft={{ ...draft, text: 'Keep this new draft' }}
      onDraftChange={onDraftChange}
      sessions={[target]}
    />,
  );
  await waitFor(() => expect(request).toHaveBeenCalledWith({ action: 'directions', id: 'other' }));
  const calls = onDraftChange.mock.calls.length;
  await act(async () => resolve({ action: 'prepareDirection', direction: record, created: true }));
  expect(onDraftChange).toHaveBeenCalledTimes(calls);
  expect(screen.getByLabelText('What should change?')).toHaveValue('Keep this new draft');
  expect(
    request.mock.calls.filter(([r]) => r.action === 'directions' && r.id === 'work'),
  ).toHaveLength(1);
});
