import React from 'react';
import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentCompletion from '../../src/components/IntentCompletion';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';
const workspace: IntentWorkspace = {
  id: 'w',
  projectRoot: '/project',
  title: 'Export',
  outcome: 'CSV',
  constraints: '',
  successCriteria: 'CSV opens',
  sourceUrl: '',
  status: 'review',
  revision: 1,
  createdAt: '2026-09-14',
  updatedAt: '2026-09-14',
};
const proposal = {
  id: 'proposal',
  workspaceId: 'w',
  intentRevision: 1,
  executionId: 'e',
  runId: 'r',
  session: { sessionId: 's', hub: '', label: 'Manager', provider: 'codex', cwd: '/project' },
  completedAt: '2026-09-14',
  capturedAt: '2026-09-14',
  report: 'Actual final report\nNo invented checks.',
  reportState: 'reported',
  checks: ['12 tests passed'],
  artifacts: ['export.ts'],
  provenance: 'owner-host-final-assistant/v1',
  redacted: false,
};
const api = vi.fn();
let evidence: any, view: any, reviews: any[], directions: any[];
beforeEach(() => {
  api.mockReset();
  reviews = [];
  directions = [];
  evidence = { criteria: [{ id: 'r1:c1', intentRevision: 1, text: 'CSV opens' }], evidence: [] };
  view = { action: 'completionProposals', proposals: [proposal], currentProposalId: 'proposal' };
  api.mockImplementation(async (request) => {
    if (request.action === 'completionProposals') return view;
    if (request.action === 'evidence') return { action: 'evidence', ...evidence, reviews };
    if (request.action === 'directions') return { action: 'directions', directions };
    if (request.action === 'recordReview') {
      reviews = [{ ...request, intentRevision: 1 }];
      return { action: 'recordReview', review: reviews[0] };
    }
  });
  Object.assign(window.electronAPI, { intentWorkspaceRequest: api });
});
const show = () => {
  const onEvidence = vi.fn(),
    onOpenSession = vi.fn();
  render(
    <IntentCompletion
      workspace={workspace}
      disabled={false}
      onChanged={vi.fn()}
      onEvidence={onEvidence}
      onOpenSession={onOpenSession}
    />,
  );
  return { onEvidence, onOpenSession };
};
it('shows attention and report with accessible explicit actions and evidence gate', async () => {
  const hooks = show();
  await screen.findByRole('heading', { name: 'Review needed' });
  expect(screen.getByText('12 tests passed')).toBeVisible();
  expect(screen.getByText('export.ts')).toBeVisible();
  expect(screen.getByRole('button', { name: 'Approve outcome' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: 'Select or verify evidence in Review' }));
  expect(hooks.onEvidence).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole('button', { name: 'Open execution session' }));
  expect(hooks.onOpenSession).toHaveBeenCalledWith(proposal.session);
});
it('accepts only selected user verification and reports Approved', async () => {
  evidence.evidence = [
    {
      id: 'verified',
      intentRevision: 1,
      criterion: { id: 'r1:c1' },
      assessment: 'user-verified',
      note: 'I checked CSV',
    },
  ];
  show();
  await screen.findByRole('heading', { name: 'Review needed' });
  fireEvent.change(screen.getByLabelText('Outcome review feedback'), {
    target: { value: 'I verified export' },
  });
  expect(screen.getByRole('button', { name: 'Approve outcome' })).toBeDisabled();
  fireEvent.click(screen.getByRole('checkbox', { name: 'I checked CSV' }));
  fireEvent.click(screen.getByRole('button', { name: 'Approve outcome' }));
  await screen.findByRole('heading', { name: 'Approved' });
  expect(api).toHaveBeenCalledWith(
    expect.objectContaining({
      action: 'recordReview',
      proposalId: 'proposal',
      expectedRevision: 1,
      evidenceIds: ['verified'],
      decision: 'accept',
    }),
  );
});
it('retries an ambiguous changes request with exactly the same operation', async () => {
  const base = api.getMockImplementation()!;
  api.mockImplementation(async (input) => {
    if (input.action === 'recordReview') throw new Error('Lost response');
    return base(input);
  });
  show();
  await screen.findByRole('heading', { name: 'Review needed' });
  fireEvent.change(screen.getByLabelText('Outcome review feedback'), {
    target: { value: 'Handle Unicode' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Request changes' }));
  await screen.findByRole('alert');
  const first = api.mock.calls.find(([input]) => input.action === 'recordReview')![0];
  fireEvent.click(screen.getByRole('button', { name: 'Request changes' }));
  await waitFor(() =>
    expect(api.mock.calls.filter(([input]) => input.action === 'recordReview')).toHaveLength(2),
  );
  expect(api.mock.calls.filter(([input]) => input.action === 'recordReview')[1][0]).toEqual(first);
});
it('keeps stale proposals inspectable without current revision actions', async () => {
  view.currentProposalId = null;
  view.proposals = [{ ...proposal, intentRevision: 0 }];
  show();
  await screen.findByText('Earlier completion reports');
  expect(screen.queryByRole('button', { name: 'Approve outcome' })).not.toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Request changes' })).not.toBeInTheDocument();
});
it('shows malformed report inspection and blocks approval', async () => {
  view.proposals = [{ ...proposal, completedAt: null, reportState: 'malformed' }];
  show();
  await screen.findByRole('heading', { name: 'Needs inspection' });
  expect(screen.getByRole('button', { name: 'Approve outcome' })).toBeDisabled();
});
it.each(['accepted', 'failed', 'unknown'])('displays %s delivery honestly', async (status) => {
  reviews = [
    {
      proposalId: 'proposal',
      intentRevision: 1,
      decision: 'changes-requested',
      reason: 'Unicode',
      directionId: 'direction',
    },
  ];
  directions = [{ id: 'direction', attempts: [{ status, detail: 'rejected' }] }];
  view.currentProposalId = null;
  show();
  await screen.findByRole('heading', { name: 'Changes requested' });
  expect(screen.getByRole('status').textContent).toMatch(
    status === 'accepted'
      ? /consumption is not confirmed/
      : status === 'failed'
        ? /rejected/
        : /unknown/,
  );
});
