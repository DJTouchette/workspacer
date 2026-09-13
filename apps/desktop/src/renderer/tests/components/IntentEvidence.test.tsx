import React, { useState } from 'react';
import { beforeEach, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import IntentEvidence, { type IntentEvidenceDraft } from '../../src/components/IntentEvidence';
import {
  intentCriteria,
  type IntentEvidence as Evidence,
  type IntentEvidenceResponse,
} from '../../../main/shared/intentEvidence';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';
const workspace: IntentWorkspace = {
  id: 'work',
  title: 'Export',
  projectRoot: '/project',
  revision: 2,
  outcome: 'Useful exports',
  constraints: '',
  successCriteria: 'Valid CSV\nWorks offline',
  sourceUrl: '',
  status: 'review',
  createdAt: '',
  updatedAt: '',
};
const criteria = intentCriteria(workspace.revision, workspace.successCriteria);
const draft: IntentEvidenceDraft = {
  criterionId: '',
  executionId: '',
  note: '',
  reference: '',
  assessment: 'reported',
  reviewDecision: 'changes-requested',
  reviewReason: '',
  reviewEvidenceIds: [],
};
const base: Evidence = {
  id: 'reported',
  workspaceId: 'work',
  intentRevision: 2,
  criterion: criteria[0],
  kind: 'manual',
  author: 'user',
  assessment: 'reported',
  note: 'Agent reported successful CSV export.',
  reference: '',
  createdAt: '2026-09-13T00:00:00Z',
};
const git: Evidence = {
  ...base,
  id: 'git',
  kind: 'git',
  note: 'Host-captured Git snapshot. Criterion satisfaction has not been verified.',
  executionId: 'run',
  git: {
    cwd: '/host/actual',
    repositoryRoot: '/host/actual',
    headCommit: 'a'.repeat(40),
    capturedAt: '2026-09-13T00:00:00Z',
    scope: 'tracked-working-tree-against-head',
    changedFiles: ['export.ts'],
    omissions: ['Untracked file content omitted: scratch.txt'],
    artifactId: 'artifact',
    sha256: 'b'.repeat(64),
    bytes: 42,
  },
};
type Listing = Extract<IntentEvidenceResponse, { action: 'evidence' }>;
let listing: Listing;
const request = vi.fn();
it('explains why remote execution Git capture is unavailable while allowing manual evidence', async () => {
  const original = request.getMockImplementation()!;
  request.mockImplementation(async (input) => {
    const result = await original(input);
    if (input.action === 'executions') result.executions[0].session.hub = 'peer';
    return result;
  });
  render(
    <View
      initial={{ ...draft, criterionId: 'r2:c1', executionId: 'run', note: 'Remote test result' }}
    />,
  );
  await screen.findByText(/Git capture is available for executions on this host/);
  expect(screen.getByRole('button', { name: 'Capture Git evidence' })).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Record evidence' })).toBeEnabled();
});
it('reuses Git capture identity after a lost response while preserving notes through navigation', async () => {
  const original = request.getMockImplementation()!;
  let failed = false;
  request.mockImplementation(async (input) => {
    if (input.action === 'captureEvidence' && !failed) {
      failed = true;
      throw new Error('Lost capture response');
    }
    return original(input);
  });
  const rendered = render(
    <View
      initial={{ ...draft, criterionId: 'r2:c1', executionId: 'run', note: 'Preserve my notes' }}
    />,
  );
  await screen.findByRole('option', { name: 'Worker · This host' });
  fireEvent.click(screen.getByRole('button', { name: 'Capture Git evidence' }));
  await screen.findByRole('alert');
  const first = request.mock.calls.find(([input]) => input.action === 'captureEvidence')![0];
  rendered.rerender(<View visible={false} />);
  rendered.rerender(<View />);
  fireEvent.click(screen.getByRole('button', { name: 'Capture Git evidence' }));
  await waitFor(() =>
    expect(request.mock.calls.filter(([input]) => input.action === 'captureEvidence')).toHaveLength(
      2,
    ),
  );
  expect(request.mock.calls.filter(([input]) => input.action === 'captureEvidence')[1][0]).toEqual(
    first,
  );
  expect(screen.getByLabelText('Evidence or verification notes')).toHaveValue('Preserve my notes');
});
it('keeps review retry pinned to its original revision after saved intent advances', async () => {
  const original = request.getMockImplementation()!;
  request.mockImplementation(async (input) => {
    if (input.action === 'recordReview') throw new Error('Review response lost');
    return original(input);
  });
  const rendered = render(<View initial={{ ...draft, reviewReason: 'Needs tests' }} />);
  await screen.findByRole('option', { name: 'Valid CSV' });
  fireEvent.click(screen.getByRole('button', { name: 'Record review decision' }));
  await screen.findByRole('alert');
  const first = request.mock.calls.find(([input]) => input.action === 'recordReview')![0];
  rendered.rerender(<View record={{ ...workspace, revision: 3 }} />);
  fireEvent.click(screen.getByRole('button', { name: 'Record review decision' }));
  await waitFor(() =>
    expect(request.mock.calls.filter(([input]) => input.action === 'recordReview')).toHaveLength(2),
  );
  expect(request.mock.calls.filter(([input]) => input.action === 'recordReview')[1][0]).toEqual(
    first,
  );
});
beforeEach(() => {
  listing = { action: 'evidence', criteria, evidence: [], reviews: [] };
  request.mockReset().mockImplementation(async (input) => {
    if (input.action === 'evidence') return structuredClone(listing);
    if (input.action === 'executions')
      return {
        action: 'executions',
        executions: [
          {
            id: 'run',
            session: {
              sessionId: 'worker',
              label: 'Worker',
              hub: '',
              cwd: '/renderer/untrusted',
              provider: 'codex',
            },
          },
        ],
        links: [],
      };
    if (input.action === 'readEvidence')
      return {
        action: 'readEvidence',
        evidence: git,
        artifact: 'diff --git a/export.ts b/export.ts\n+return CSV;',
      };
    if (input.action === 'addEvidence') {
      const evidence = {
        ...base,
        id: input.evidenceId,
        criterion: criteria.find((c) => c.id === input.criterionId)!,
        note: input.note,
        reference: input.reference,
        assessment: input.assessment,
        linkedEvidenceId: input.linkedEvidenceId,
      };
      listing.evidence.unshift(evidence);
      return { action: 'addEvidence', evidence };
    }
    if (input.action === 'captureEvidence') {
      listing.evidence.unshift({ ...git, id: input.evidenceId });
      return { action: 'captureEvidence', evidence: listing.evidence[0] };
    }
    if (input.action === 'recordReview') {
      const review = {
        id: input.reviewId,
        workspaceId: 'work',
        intentRevision: input.expectedRevision,
        author: 'user' as const,
        decision: input.decision,
        reason: input.reason,
        evidenceIds: input.evidenceIds,
        createdAt: '2026-09-13T00:00:00Z',
      };
      listing.reviews.unshift(review);
      return { action: 'recordReview', review };
    }
    throw new Error('Unexpected request');
  });
  Object.assign(window.electronAPI, { intentWorkspaceRequest: request });
});
function View({
  visible = true,
  disabled = false,
  record = workspace,
  initial = draft,
}: {
  visible?: boolean;
  disabled?: boolean;
  record?: IntentWorkspace;
  initial?: IntentEvidenceDraft;
} = {}) {
  const [value, setValue] = useState(initial);
  return (
    <IntentEvidence
      workspace={record}
      visible={visible}
      disabled={disabled}
      draft={value}
      onDraftChange={setValue}
    />
  );
}
const selectForReview = (id: string) => {
  const index = listing.evidence.findIndex((e) => e.id === id);
  fireEvent.click(screen.getAllByLabelText('Include in review')[index]);
};
it('keeps host Git facts distinct from criterion verification and records verification only after explicit user input', async () => {
  listing.evidence = [git];
  render(<View />);
  await screen.findByText(/Git facts captured by host/);
  expect(screen.getByText(/Evidence needs review/)).toBeInTheDocument();
  expect(screen.queryByText(/User verification recorded/)).not.toBeInTheDocument();
  expect(request.mock.calls.every(([r]) => ['evidence', 'executions'].includes(r.action))).toBe(
    true,
  );
  fireEvent.click(screen.getByRole('button', { name: 'Inspect captured diff' }));
  expect(await screen.findByText(/\+return CSV;/)).toBeInTheDocument();
  expect(request).toHaveBeenCalledWith({ action: 'readEvidence', id: 'work', evidenceId: 'git' });
  expect(screen.queryByText(/User verification recorded/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Record my verification' }));
  expect(screen.getByLabelText('Evidence assessment')).toHaveValue('user-verified');
  expect(screen.getByRole('button', { name: 'Record evidence' })).toBeDisabled();
  expect(request.mock.calls.some(([r]) => r.action === 'addEvidence')).toBe(false);
  fireEvent.change(screen.getByLabelText('Evidence or verification notes'), {
    target: { value: 'I opened the CSV and verified headers and rows.' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Record evidence' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'addEvidence',
        criterionId: 'r2:c1',
        assessment: 'user-verified',
        linkedEvidenceId: 'git',
        expectedRevision: 2,
      }),
    ),
  );
  expect(await screen.findByText(/User verification recorded/)).toBeInTheDocument();
  expect(screen.getByText(/Git facts captured by host/)).toHaveTextContent('Reported');
});
it('requires selected user verification for every criterion and refuses selected unresolved evidence before acceptance', async () => {
  listing.evidence = [
    { ...base, id: 'verified-csv', assessment: 'user-verified', note: 'I verified CSV' },
    {
      ...base,
      id: 'verified-offline',
      criterion: criteria[1],
      assessment: 'user-verified',
      note: 'I verified offline',
    },
    {
      ...base,
      id: 'blocker',
      criterion: criteria[1],
      assessment: 'unresolved',
      note: 'An offline error still needs a decision',
    },
  ];
  render(<View />);
  await screen.findByText('I verified CSV');
  fireEvent.change(screen.getByLabelText('Review outcome'), { target: { value: 'accept' } });
  fireEvent.change(screen.getByLabelText('Review reason'), {
    target: { value: 'Checked both behaviors on the saved revision.' },
  });
  const save = screen.getByRole('button', { name: 'Record review decision' });
  expect(save).toBeDisabled();
  selectForReview('verified-csv');
  expect(save).toBeDisabled();
  selectForReview('verified-offline');
  expect(save).toBeEnabled();
  selectForReview('blocker');
  expect(save).toBeDisabled();
  selectForReview('blocker');
  fireEvent.click(save);
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'recordReview',
        decision: 'accept',
        expectedRevision: 2,
        evidenceIds: ['verified-csv', 'verified-offline'],
      }),
    ),
  );
  expect(await screen.findByText('Review accepted')).toBeInTheDocument();
  expect(screen.getByText('An offline error still needs a decision')).toBeInTheDocument();
});
it('retains evidence drafts and request identity after stale-write errors and tab navigation', async () => {
  const rendered = render(<View />);
  await screen.findByRole('option', { name: 'Valid CSV' });
  fireEvent.change(screen.getByLabelText('Success criterion'), { target: { value: 'r2:c1' } });
  fireEvent.change(screen.getByLabelText('Evidence or verification notes'), {
    target: { value: 'The data still needs checking.' },
  });
  request.mockImplementationOnce(async () => {
    throw new Error('Intent changed elsewhere. Reload before recording evidence or review.');
  });
  fireEvent.click(screen.getByRole('button', { name: 'Record evidence' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Intent changed elsewhere');
  const first = request.mock.calls.find(([r]) => r.action === 'addEvidence')![0];
  rendered.rerender(<View visible={false} />);
  rendered.rerender(<View />);
  expect(screen.getByLabelText('Evidence or verification notes')).toHaveValue(
    'The data still needs checking.',
  );
  fireEvent.click(screen.getByRole('button', { name: 'Record evidence' }));
  await waitFor(() =>
    expect(request.mock.calls.filter(([r]) => r.action === 'addEvidence')).toHaveLength(2),
  );
  const second = request.mock.calls.filter(([r]) => r.action === 'addEvidence')[1][0];
  expect(second.evidenceId).toBe(first.evidenceId);
  expect(second.expectedRevision).toBe(2);
});
it('preserves review reason and selection when the host refuses a stale review', async () => {
  listing.evidence = [base];
  render(<View />);
  await screen.findByText(base.note);
  selectForReview(base.id);
  fireEvent.change(screen.getByLabelText('Review reason'), {
    target: { value: 'The report needs an actual test.' },
  });
  request.mockImplementationOnce(async () => {
    throw new Error('Intent changed elsewhere. Reload before recording evidence or review.');
  });
  fireEvent.click(screen.getByRole('button', { name: 'Record review decision' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Intent changed elsewhere');
  expect(screen.getByLabelText('Review reason')).toHaveValue('The report needs an actual test.');
  expect(screen.getByLabelText('Include in review')).toBeChecked();
  expect(screen.queryByText('Changes requested')).not.toBeInTheDocument();
});
it('does not reuse verification from an earlier intent and disables writes for unsaved intent edits', async () => {
  listing.evidence = [
    {
      ...base,
      id: 'old',
      intentRevision: 1,
      criterion: { id: 'r1:c1', intentRevision: 1, text: 'Valid CSV' },
      assessment: 'user-verified',
    },
  ];
  render(
    <View
      disabled
      initial={{
        ...draft,
        reviewDecision: 'accept',
        reviewReason: 'Review',
        reviewEvidenceIds: ['old'],
      }}
    />,
  );
  await screen.findByText(/Revision 1 · Earlier intent/);
  expect(screen.queryByText(/User verification recorded/)).not.toBeInTheDocument();
  expect(screen.queryByLabelText('Include in review')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Record review decision' })).toBeDisabled();
  expect(
    within(screen.getByRole('region', { name: 'Evidence and review' })).getByText(
      /Save the intent before adding evidence/,
    ),
  ).toBeInTheDocument();
});

it('lets a current-revision refresh finish while an older saved diff is being inspected', async () => {
  listing.evidence = [git];
  const rendered = render(<View />);
  await screen.findByText(/Git facts captured by host/);
  let resolve!: (value: Listing) => void;
  const refresh = new Promise<Listing>((done) => {
    resolve = done;
  });
  const previous = request.getMockImplementation()!;
  request.mockImplementation((input) => (input.action === 'evidence' ? refresh : previous(input)));
  const next = { ...workspace, revision: 3, successCriteria: 'New current criterion' };
  rendered.rerender(<View record={next} />);
  fireEvent.click(screen.getByRole('button', { name: 'Inspect captured diff' }));
  await screen.findByText(/\+return CSV;/);
  await act(async () => resolve({ ...listing, criteria: intentCriteria(3, next.successCriteria) }));
  expect(
    await screen.findByText('New current criterion', { selector: 'strong' }),
  ).toBeInTheDocument();
});
