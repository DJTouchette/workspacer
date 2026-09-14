import React from 'react';
import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentAutomation from '../../src/components/IntentAutomation';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';
import type { IntentRun } from '../../../main/shared/intentAutomation';
const workspace: IntentWorkspace = {
  id: 'work',
  projectRoot: '/project',
  title: 'CSV',
  outcome: 'Export',
  constraints: '',
  successCriteria: 'Valid CSV',
  sourceUrl: '',
  status: 'draft',
  revision: 1,
  createdAt: '2026-09-13',
  updatedAt: '2026-09-13',
};
const run: IntentRun = {
  id: 'run',
  workspaceId: 'work',
  executionId: 'execution',
  intentRevision: 1,
  state: 'waiting',
  session: { sessionId: 'manager', hub: '', label: 'Manager', cwd: '/project', provider: 'codex' },
  message: '',
  report: 'All rows or current page?',
  deadline: '2026-09-14T12:00:00Z',
  updatedAt: '2026-09-13',
};
const api = vi.fn();
beforeEach(() => {
  api.mockReset();
  Object.assign(window.electronAPI, { intentWorkspaceRequest: api });
});
it('activates saved work with a visible work limit', async () => {
  api.mockImplementation(async (input) => ({
    action: input.action,
    run: input.action === 'automation' ? null : { ...run, state: 'queued' },
  }));
  const changed = vi.fn();
  render(<IntentAutomation workspace={workspace} disabled={false} onChanged={changed} />);
  await screen.findByRole('button', { name: 'Activate' });
  fireEvent.change(screen.getByLabelText('Work limit (minutes)'), { target: { value: '30' } });
  fireEvent.click(screen.getByRole('button', { name: 'Activate' }));
  await waitFor(() =>
    expect(api).toHaveBeenCalledWith({
      action: 'activateIntent',
      id: 'work',
      expectedRevision: 1,
      minutes: 30,
    }),
  );
  expect(changed).toHaveBeenCalledOnce();
});
it('answers a manager question and resumes without opening a launch dialog', async () => {
  api.mockImplementation(async (input) => ({
    action: input.action,
    run: input.action === 'automation' ? run : { ...run, id: 'next', state: 'queued' },
  }));
  render(<IntentAutomation workspace={workspace} disabled={false} onChanged={vi.fn()} />);
  await screen.findByText('All rows or current page?');
  fireEvent.change(screen.getByLabelText('Your answer'), {
    target: { value: 'All filtered rows' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Answer and continue' }));
  await waitFor(() =>
    expect(api).toHaveBeenCalledWith({
      action: 'answerIntent',
      id: 'work',
      runId: 'run',
      expectedRevision: 1,
      text: 'All filtered rows',
    }),
  );
});
it('keeps native approvals in their existing question UI and disables work controls for unsaved edits', async () => {
  api.mockResolvedValue({ action: 'automation', run });
  const open = vi.fn();
  render(
    <IntentAutomation
      workspace={workspace}
      disabled={true}
      onChanged={vi.fn()}
      onOpenSession={open}
      sessions={[{ ...run.session!, pendingApproval: { toolName: 'shell' } }]}
    />,
  );
  const button = await screen.findByRole('button', { name: 'Answer agent question or approval' });
  expect(screen.queryByLabelText('Your answer')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Pause work' })).toBeDisabled();
  fireEvent.click(button);
  expect(open).toHaveBeenCalledWith(run.session);
});
