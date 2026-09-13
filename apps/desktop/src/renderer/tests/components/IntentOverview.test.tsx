import React from 'react';
import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentOverview, { IntentAttentionBadge } from '../../src/components/IntentOverview';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';
const workspace: IntentWorkspace = {
  id: 'work',
  projectRoot: '/project',
  revision: 2,
  title: 'Feature',
  outcome: 'Export filtered rows',
  constraints: '',
  successCriteria: 'Preserve permissions',
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
const request = vi.fn(),
  navigate = vi.fn(),
  openSession = vi.fn();
beforeEach(() => {
  navigate.mockReset();
  openSession.mockReset();
  request.mockReset().mockImplementation(async (input) => {
    if (input.action === 'executions')
      return {
        action: 'executions',
        executions: [
          {
            id: 'run',
            workspaceId: 'work',
            intentRevision: 2,
            state: 'linked',
            session: target,
            lastObservation: {
              state: 'idle',
              summary: 'Everything is complete',
              observedAt: '2026-09-12T12:00:00Z',
              cwd: '/project',
            },
          },
        ],
        links: [],
      };
    if (input.action === 'evidence')
      return { action: 'evidence', criteria: [], evidence: [], reviews: [] };
    if (input.action === 'directions') return { action: 'directions', directions: [] };
    if (input.action === 'sources') return { action: 'sources', sources: [], comments: [] };
    if (input.action === 'artifacts')
      return {
        action: 'artifacts',
        artifacts: [],
        annotations: [],
        demonstrations: [],
        groups: [],
        selections: [],
      };
    throw new Error('Unexpected request');
  });
  Object.assign(window.electronAPI, {
    intentWorkspaceRequest: request,
    claudeApprove: vi.fn(),
    claudeAnswer: vi.fn(),
  });
});
it('shows retained reports separately from user review and navigates uncovered criteria to evidence', async () => {
  render(<IntentOverview workspace={workspace} visible onNavigate={navigate} />);
  expect(await screen.findByText('Everything is complete')).toBeInTheDocument();
  expect(screen.getByText('Your review: Not recorded for this revision')).toBeInTheDocument();
  expect(
    screen.getByText('0 of 1 criteria have user-verified evidence for this revision.'),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Add criterion evidence' }));
  expect(navigate).toHaveBeenCalledWith('review');
  expect(request.mock.calls.map(([input]) => input.action).sort()).toEqual([
    'artifacts',
    'directions',
    'evidence',
    'executions',
    'sources',
  ]);
});
it('routes qualified linked approvals/questions to the original agent surface without resolving or dismissing them', async () => {
  const local = {
    ...target,
    pendingApproval: { toolName: 'Bash' },
    pendingQuestions: [{ question: 'Which environment?' }],
  };
  const { rerender } = render(
    <IntentOverview
      workspace={workspace}
      visible
      sessions={[local]}
      onNavigate={navigate}
      onOpenSession={openSession}
    />,
  );
  expect(await screen.findByText('Approval needed: Bash')).toBeInTheDocument();
  expect(screen.getByText('Agent question: Which environment?')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Open agent to respond' }));
  expect(openSession).toHaveBeenCalledWith(target);
  expect(window.electronAPI.claudeApprove).not.toHaveBeenCalled();
  expect(window.electronAPI.claudeAnswer).not.toHaveBeenCalled();
  expect(screen.getByText('Approval needed: Bash')).toBeInTheDocument();
  rerender(
    <IntentOverview
      workspace={workspace}
      visible
      sessions={[{ ...local, hub: 'other-host' }]}
      onNavigate={navigate}
      onOpenSession={openSession}
    />,
  );
  expect(screen.queryByText('Approval needed: Bash')).not.toBeInTheDocument();
});
it('shows partial read failures while retaining independently loaded results and makes no hidden read requests', async () => {
  const normal = request.getMockImplementation()!;
  request.mockImplementation(async (input) => {
    if (input.action === 'evidence') throw new Error('Storage unavailable');
    return normal(input);
  });
  const { rerender } = render(
    <IntentOverview workspace={workspace} visible={false} onNavigate={navigate} />,
  );
  expect(request).not.toHaveBeenCalled();
  rerender(<IntentOverview workspace={workspace} visible onNavigate={navigate} />);
  expect(await screen.findByRole('alert')).toHaveTextContent(
    'Evidence and review: Error: Storage unavailable',
  );
  expect(screen.getByText('Everything is complete')).toBeInTheDocument();
  expect(screen.getByText('Evidence coverage is unavailable.')).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Add criterion evidence' })).not.toBeInTheDocument();
  rerender(<IntentOverview workspace={workspace} visible={false} onNavigate={navigate} />);
  await waitFor(() => expect(request).toHaveBeenCalledTimes(5));
});
it('keeps source drift and alternative selection visible as deliberate next decisions', async () => {
  const normal = request.getMockImplementation()!;
  request.mockImplementation(async (input) => {
    if (input.action === 'sources')
      return {
        action: 'sources',
        sources: [{ accepted: { digest: 'old' }, candidate: { digest: 'new' } }],
        comments: [],
      };
    if (input.action === 'artifacts')
      return {
        action: 'artifacts',
        artifacts: [],
        annotations: [],
        demonstrations: [],
        selections: [],
        groups: [
          {
            id: 'alternatives',
            title: 'Compare layouts',
            intentRevision: 2,
            alternatives: [
              { id: 'a', title: 'Grid' },
              { id: 'b', title: 'Rows' },
            ],
          },
        ],
      };
    return normal(input);
  });
  render(<IntentOverview workspace={workspace} visible onNavigate={navigate} />);
  fireEvent.click(await screen.findByRole('button', { name: 'Review source changes' }));
  fireEvent.click(screen.getByRole('button', { name: 'Compare alternatives' }));
  expect(navigate.mock.calls).toEqual([['sources'], ['artifacts']]);
  expect(screen.getByText('No selection recorded for this revision')).toBeInTheDocument();
});
it('shows sidebar attention count for unique qualified live links only', () => {
  const local = { ...target, pendingApproval: { toolName: 'Edit' } };
  const { rerender } = render(<IntentAttentionBadge refs={[target, target]} sessions={[local]} />);
  expect(screen.getByLabelText('1 linked agents need attention')).toBeInTheDocument();
  rerender(<IntentAttentionBadge refs={[target]} sessions={[{ ...local, hubOffline: true }]} />);
  expect(screen.queryByLabelText('1 linked agents need attention')).not.toBeInTheDocument();
});
