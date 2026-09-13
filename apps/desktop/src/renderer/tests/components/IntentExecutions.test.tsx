import React from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentExecutions from '../../src/components/IntentExecutions';
import type { IntentExecution, IntentWorkspace } from '../../../main/shared/intentWorkspace';

const workspace: IntentWorkspace = {
  id: 'intent',
  title: 'Export',
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
const session = {
  sessionId: 'session',
  hub: 'peer',
  label: 'Export agent',
  provider: 'codex',
  cwd: '/project',
};
const execution: IntentExecution = {
  id: 'run',
  workspaceId: 'intent',
  intentRevision: 1,
  kind: 'launch',
  state: 'linked',
  task: 'Implement',
  contextPacket: 'Pinned context',
  session,
  lastObservation: {
    state: 'idle',
    summary: 'Previous result',
    cwd: '/project',
    observedAt: '2026-09-12T12:00:00Z',
  },
  createdAt: '',
  updatedAt: '',
};
const request = vi.fn();
beforeEach(() => {
  request.mockReset();
  request.mockResolvedValue({ action: 'executions', executions: [], links: [] });
  Object.assign(window.electronAPI, { intentWorkspaceRequest: request, claudeMessage: vi.fn() });
});

describe('feature execution surface', () => {
  it('shows a retained report alongside a sparse live headless status', async () => {
    request.mockResolvedValue({ action: 'executions', executions: [execution], links: [] });
    render(
      <IntentExecutions workspace={workspace} sessions={[{ ...session, ambientState: 'idle' }]} />,
    );
    expect(await screen.findByText('Previous result')).toBeInTheDocument();
    expect(screen.getByText('Saved agent report excerpt')).toBeInTheDocument();
  });
  it('shows background capture failures alongside the previously retained report', async () => {
    request.mockResolvedValue({
      action: 'executions',
      executions: [execution],
      links: [],
      captureWarning: 'Background result capture failed: disk full',
    });
    render(<IntentExecutions workspace={workspace} />);
    expect(await screen.findByRole('alert')).toHaveTextContent('disk full');
    expect(screen.getByText('Previous result')).toBeInTheDocument();
    request.mockResolvedValue({ action: 'executions', executions: [execution], links: [] });
    fireEvent.click(screen.getByRole('button', { name: 'Refresh execution' }));
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
  });
  it('links an existing agent without sending any message and preserves hub identity', async () => {
    render(<IntentExecutions workspace={workspace} candidates={[session]} />);
    fireEvent.change(screen.getByLabelText('Existing agent'), {
      target: { value: JSON.stringify(['peer', 'session']) },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Link agent' }));
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith({
        action: 'attachSession',
        id: 'intent',
        expectedRevision: 2,
        session,
      }),
    );
    expect(window.electronAPI.claudeMessage).not.toHaveBeenCalled();
  });

  it('requires saved intent for launching and shows the revision used by previous work', async () => {
    request.mockResolvedValue({ action: 'executions', executions: [execution], links: [] });
    render(<IntentExecutions workspace={workspace} disabled onStart={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'Start agent' })).toBeDisabled();
    expect(await screen.findByText('Revision 1 · Earlier intent')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Context recorded at launch'));
    expect(screen.getByText('Pinned context')).toBeInTheDocument();
  });

  it('shows retained observations when a peer is offline and does not open a different host session', async () => {
    request.mockResolvedValue({ action: 'executions', executions: [execution], links: [] });
    render(
      <IntentExecutions
        workspace={workspace}
        candidates={[{ ...session, hub: '' }]}
        sessions={[{ ...session, hubOffline: true, ambientState: 'thinking' }]}
        onOpenSession={vi.fn()}
      />,
    );
    expect(await screen.findByText('Previous result')).toBeInTheDocument();
    expect(screen.getByText(/Not currently observed/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open agent' })).toBeDisabled();
  });

  it('keeps a failed reference draft and reports the error', async () => {
    request.mockImplementation(async (input) => {
      if (input.action === 'addWorkLink') throw new Error('Database unavailable');
      return { action: 'executions', executions: [], links: [] };
    });
    render(<IntentExecutions workspace={workspace} />);
    fireEvent.change(screen.getByLabelText('Pull request URL'), {
      target: { value: 'https://example.com/pr/1' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add reference' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Database unavailable');
    expect(screen.getByLabelText('Pull request URL')).toHaveValue('https://example.com/pr/1');
    fireEvent.click(screen.getByRole('button', { name: 'Refresh execution' }));
    await waitFor(() =>
      expect(
        request.mock.calls.filter(([input]) => input.action === 'executions').length,
      ).toBeGreaterThan(1),
    );
    expect(screen.getByRole('alert')).toHaveTextContent('Database unavailable');
  });
});
