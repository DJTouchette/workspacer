import React, { useState } from 'react';
import { beforeEach, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentKnowledge, { type IntentKnowledgeDraft } from '../../src/components/IntentKnowledge';
import type {
  IntentFinding,
  IntentKnowledgeCapture,
  IntentKnowledgePromotion,
  IntentKnowledgeResponse,
} from '../../../main/shared/intentKnowledge';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';
const workspace: IntentWorkspace = {
  id: 'work',
  title: 'Export',
  projectRoot: '/project',
  revision: 2,
  outcome: '',
  constraints: '',
  successCriteria: 'Works offline',
  sourceUrl: '',
  status: 'active',
  createdAt: '',
  updatedAt: '',
};
const draft: IntentKnowledgeDraft = {
  title: '',
  observation: '',
  captureIds: [],
  proposalFindingId: '',
  proposalKind: 'learning',
  proposalPath: '',
};
const capture: IntentKnowledgeCapture = {
  id: 'capture-old',
  workspaceId: 'work',
  intentRevision: 1,
  path: '.rivet/context/modules/export.md',
  title: 'Export conventions',
  sha256: 'a'.repeat(64),
  bytes: 40,
  content: '# Export conventions\n\nOriginal guidance.',
  capturedAt: '2026-09-12T00:00:00Z',
};
const finding: IntentFinding = {
  id: 'finding',
  workspaceId: 'work',
  intentRevision: 2,
  title: 'Reuse CSV escaping',
  observation: 'The existing helper handles quoted fields correctly.',
  captureIds: [capture.id],
  author: 'user',
  createdAt: '2026-09-13T00:00:00Z',
};
const proposal: IntentKnowledgePromotion = {
  id: 'proposal',
  workspaceId: 'work',
  findingId: finding.id,
  intentRevision: 2,
  projectRoot: '/project',
  kind: 'learning',
  path: '.rivet/learnings/2026-09-13-intent-proposal.md',
  previousSha256: null,
  content: '# Reuse CSV escaping\n\nExact reviewed project learning.',
  createdAt: '2026-09-13T00:00:00Z',
  status: 'draft',
  detail: 'Saved for review; no project file changed.',
};
type Listing = Extract<IntentKnowledgeResponse, { action: 'knowledge' }>;
let listing: Listing;
const request = vi.fn();
it('reuses document capture identity after a lost response and tab navigation', async () => {
  const original = request.getMockImplementation()!;
  let failed = false;
  request.mockImplementation(async (input) => {
    if (input.action === 'captureKnowledge' && !failed) {
      failed = true;
      throw new Error('Lost capture response');
    }
    return original(input);
  });
  const rendered = render(<View />);
  fireEvent.click(await screen.findByRole('button', { name: 'Capture document version' }));
  await screen.findByRole('alert');
  const first = request.mock.calls.find(([input]) => input.action === 'captureKnowledge')![0];
  rendered.rerender(<View visible={false} />);
  rendered.rerender(<View />);
  fireEvent.click(await screen.findByRole('button', { name: 'Capture document version' }));
  await waitFor(() =>
    expect(
      request.mock.calls.filter(([input]) => input.action === 'captureKnowledge'),
    ).toHaveLength(2),
  );
  expect(request.mock.calls.filter(([input]) => input.action === 'captureKnowledge')[1][0]).toEqual(
    first,
  );
});
it('does not clear an externally retained draft after its component unmounts', async () => {
  let finish!: (value: unknown) => void;
  const original = request.getMockImplementation()!;
  request.mockImplementation((input) =>
    input.action === 'recordFinding'
      ? new Promise((resolve) => {
          finish = resolve;
        })
      : original(input),
  );
  const changed = vi.fn();
  const rendered = render(
    <IntentKnowledge
      workspace={workspace}
      visible
      disabled={false}
      draft={{ ...draft, title: 'Draft finding', observation: 'Keep this draft' }}
      onDraftChange={changed}
    />,
  );
  fireEvent.click(screen.getByRole('button', { name: 'Record feature finding' }));
  expect(changed).toHaveBeenCalledTimes(1);
  rendered.unmount();
  await act(async () => finish({ action: 'recordFinding', finding }));
  expect(changed).toHaveBeenCalledTimes(1);
});
beforeEach(() => {
  listing = {
    action: 'knowledge',
    available: true,
    documents: [{ path: capture.path, title: capture.title, sha256: 'b'.repeat(64), bytes: 45 }],
    captures: [capture],
    findings: [finding],
    proposals: [],
  };
  request.mockReset().mockImplementation(async (input) => {
    if (input.action === 'knowledge') return structuredClone(listing);
    if (input.action === 'captureKnowledge') {
      const next = {
        ...capture,
        id: input.captureId,
        intentRevision: input.expectedRevision,
        sha256: input.expectedSha256,
        content: '# Export conventions\n\nUpdated guidance.',
      };
      listing.captures.unshift(next);
      return { action: 'captureKnowledge', capture: next };
    }
    if (input.action === 'recordFinding') {
      const next = {
        ...finding,
        id: input.findingId,
        title: input.title,
        observation: input.observation,
        captureIds: input.captureIds,
      };
      listing.findings.unshift(next);
      return { action: 'recordFinding', finding: next };
    }
    if (input.action === 'prepareKnowledgePromotion') {
      const next = {
        ...proposal,
        id: input.proposalId,
        findingId: input.findingId,
        kind: input.kind,
        path: input.path || proposal.path,
      };
      listing.proposals.unshift(next);
      return { action: input.action, proposal: next };
    }
    if (input.action === 'publishKnowledgePromotion') {
      listing.proposals[0] = {
        ...listing.proposals[0],
        status: 'written',
        detail: 'Reviewed content written to the project.',
      };
      return { action: input.action, proposal: listing.proposals[0] };
    }
    if (input.action === 'reconcileKnowledgePromotion') {
      listing.proposals[0] = {
        ...listing.proposals[0],
        status: 'written',
        detail: 'Host verified that the project file matches the reviewed proposal.',
      };
      return { action: input.action, proposal: listing.proposals[0] };
    }
    throw new Error('Unexpected request');
  });
  Object.assign(window.electronAPI, { intentWorkspaceRequest: request });
});
function View({
  visible = true,
  disabled = false,
  initial = draft,
}: { visible?: boolean; disabled?: boolean; initial?: IntentKnowledgeDraft } = {}) {
  const [value, setValue] = useState(initial);
  return (
    <IntentKnowledge
      workspace={workspace}
      visible={visible}
      disabled={disabled}
      draft={value}
      onDraftChange={setValue}
    />
  );
}
it('shows source drift and captures only the explicitly selected current document version', async () => {
  render(<View />);
  expect(await screen.findByText(/Changed since your last capture/)).toBeInTheDocument();
  expect(request.mock.calls.map(([r]) => r.action)).toEqual(['knowledge']);
  fireEvent.click(screen.getByRole('button', { name: 'Capture document version' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'captureKnowledge',
        id: 'work',
        expectedRevision: 2,
        path: capture.path,
        expectedSha256: 'b'.repeat(64),
      }),
    ),
  );
  expect(await screen.findByText(/Captured version is current/)).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Capture document version' })).toBeDisabled();
  fireEvent.click(screen.getByText('Captured knowledge (2)'));
  expect(screen.getByText(/Original guidance/)).toBeInTheDocument();
  expect(screen.getByText(/Updated guidance/)).toBeInTheDocument();
  expect(request.mock.calls.some(([r]) => r.action === 'publishKnowledgePromotion')).toBe(false);
});
it('keeps a feature finding and prepared proposal private until the exact content is explicitly written', async () => {
  render(<View />);
  await screen.findByRole('option', { name: finding.title });
  fireEvent.change(screen.getByLabelText('Finding title'), {
    target: { value: 'Preserve quoted data' },
  });
  fireEvent.change(screen.getByLabelText('What should future work know?'), {
    target: { value: 'Use the existing CSV helper for escaping.' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Record feature finding' }));
  await waitFor(() =>
    expect(request.mock.calls.some(([r]) => r.action === 'recordFinding')).toBe(true),
  );
  expect(
    request.mock.calls.some(
      ([r]) => r.action === 'prepareKnowledgePromotion' || r.action === 'publishKnowledgePromotion',
    ),
  ).toBe(false);
  await waitFor(() => expect(screen.getByLabelText('Finding to promote')).not.toHaveValue(''));
  fireEvent.click(screen.getByRole('button', { name: 'Prepare promotion for review' }));
  expect(await screen.findByText('Prepared · No project file changed')).toBeInTheDocument();
  expect(
    screen.getByText(
      (_, element) => element?.tagName === 'PRE' && element.textContent === proposal.content,
    ),
  ).toBeInTheDocument();
  expect(request.mock.calls.some(([r]) => r.action === 'publishKnowledgePromotion')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'Write reviewed learning file' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith({
      action: 'publishKnowledgePromotion',
      id: 'work',
      proposalId: listing.proposals[0].id,
    }),
  );
  expect(await screen.findByText('Reviewed content was written')).toBeInTheDocument();
  expect(screen.queryByText('Project file matches reviewed content')).not.toBeInTheDocument();
  expect(
    screen.queryByRole('button', { name: 'Write reviewed learning file' }),
  ).not.toBeInTheDocument();
});
it('retains finding draft, selected provenance and request identity after a stale save and tab navigation', async () => {
  const rendered = render(<View />);
  await screen.findByRole('option', { name: finding.title });
  fireEvent.change(screen.getByLabelText('Finding title'), {
    target: { value: 'Remember this constraint' },
  });
  fireEvent.change(screen.getByLabelText('What should future work know?'), {
    target: { value: 'Preserve the existing offline contract.' },
  });
  fireEvent.click(screen.getByRole('checkbox'));
  request.mockImplementationOnce(async () => {
    throw new Error('Intent changed. Reload the saved revision before continuing.');
  });
  fireEvent.click(screen.getByRole('button', { name: 'Record feature finding' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Intent changed');
  const first = request.mock.calls.find(([r]) => r.action === 'recordFinding')![0];
  rendered.rerender(<View visible={false} />);
  rendered.rerender(<View />);
  expect(screen.getByLabelText('Finding title')).toHaveValue('Remember this constraint');
  expect(screen.getByLabelText('What should future work know?')).toHaveValue(
    'Preserve the existing offline contract.',
  );
  expect(screen.getByRole('checkbox')).toBeChecked();
  fireEvent.click(screen.getByRole('button', { name: 'Record feature finding' }));
  await waitFor(() =>
    expect(request.mock.calls.filter(([r]) => r.action === 'recordFinding')).toHaveLength(2),
  );
  const second = request.mock.calls.filter(([r]) => r.action === 'recordFinding')[1][0];
  expect(second.findingId).toBe(first.findingId);
  expect(second.expectedRevision).toBe(2);
  expect(second.captureIds).toEqual([capture.id]);
});
it('retains the chosen context promotion after stale-source refusal and never auto-writes it', async () => {
  render(<View />);
  await screen.findByRole('option', { name: finding.title });
  fireEvent.change(screen.getByLabelText('Finding to promote'), { target: { value: finding.id } });
  fireEvent.change(screen.getByLabelText('Destination'), { target: { value: 'context' } });
  fireEvent.change(screen.getByLabelText('Context document'), { target: { value: capture.path } });
  request.mockImplementationOnce(async () => {
    throw new Error('Rivet document changed. Refresh and review its new version.');
  });
  fireEvent.click(screen.getByRole('button', { name: 'Prepare promotion for review' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Rivet document changed');
  expect(screen.getByLabelText('Finding to promote')).toHaveValue(finding.id);
  expect(screen.getByLabelText('Destination')).toHaveValue('context');
  expect(screen.getByLabelText('Context document')).toHaveValue(capture.path);
  expect(request.mock.calls.some(([r]) => r.action === 'publishKnowledgePromotion')).toBe(false);
});
it('refreshes uncertain write receipts and reconciles through a read-only hash check without replaying the write', async () => {
  listing.proposals = [proposal];
  render(<View />);
  await screen.findByRole('button', { name: 'Write reviewed learning file' });
  request.mockImplementationOnce(async () => {
    listing.proposals[0] = {
      ...proposal,
      status: 'unknown',
      detail: 'File write started; completion has not been recorded.',
    };
    throw new Error('Promotion outcome needs inspection: receipt write failed.');
  });
  fireEvent.click(screen.getByRole('button', { name: 'Write reviewed learning file' }));
  expect(await screen.findByText('Write outcome uncertain')).toBeInTheDocument();
  expect(screen.getByRole('alert')).toHaveTextContent('needs inspection');
  expect(
    screen.queryByRole('button', { name: 'Write reviewed learning file' }),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Verify file against proposal' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith({
      action: 'reconcileKnowledgePromotion',
      id: 'work',
      proposalId: 'proposal',
    }),
  );
  expect(await screen.findByText('Reviewed content was written')).toBeInTheDocument();
  expect(request.mock.calls.filter(([r]) => r.action === 'publishKnowledgePromotion')).toHaveLength(
    1,
  );
});
it('keeps an unmatched uncertain write unresolved and disables publishing while intent edits are unsaved', async () => {
  listing.proposals = [{ ...proposal, status: 'unknown' }];
  render(<View disabled />);
  await screen.findByText('Write outcome uncertain');
  request.mockImplementationOnce(async () => ({
    action: 'reconcileKnowledgePromotion',
    proposal: listing.proposals[0],
  }));
  fireEvent.click(screen.getByRole('button', { name: 'Verify file against proposal' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith({
      action: 'reconcileKnowledgePromotion',
      id: 'work',
      proposalId: 'proposal',
    }),
  );
  expect(screen.getByText('Write outcome uncertain')).toBeInTheDocument();
  expect(screen.queryByText('Reviewed content was written')).not.toBeInTheDocument();
  expect(
    screen.queryByRole('button', { name: 'Write reviewed learning file' }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Record feature finding' })).toBeDisabled();
  expect(request.mock.calls.some(([r]) => r.action === 'publishKnowledgePromotion')).toBe(false);
});
