import React from 'react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor, cleanup } from '@testing-library/react';
import { ConversationMessage } from '../src/components/claude/ConversationMessage';
import { HtmlCardHostProvider } from '../src/components/claude/HtmlResponseCard';
import { buildFleetMessage } from '../../main/shared/fleetMessages';
import type { FleetReviewEvidence } from '../../main/shared/fleetReview';
const id = '11111111-1111-4111-8111-111111111111';
const evidence: FleetReviewEvidence = {
  id,
  ownerSessionId: 'manager',
  workerSessionId: 'worker',
  projectRoot: '/project',
  allocatedCwd: '/trees/worker',
  branch: 'wks/worker',
  baseCommit: 'a'.repeat(40),
  headCommit: 'b'.repeat(40),
  lifecycle: 'turn-ended',
  capturedAt: '2026-09-06T00:00:00Z',
  availability: 'captured',
  files: [{ path: 'changed.ts', status: 'M' }],
};
function mount(evidenceId: string | undefined = id, owner = 'manager') {
  return render(
    <HtmlCardHostProvider value={{ sessionId: owner, paneId: 'pane', cwd: '/manager-unrelated' }}>
      <ConversationMessage
        turn={{
          role: 'user',
          content: buildFleetMessage('worker-finished', [
            {
              sessionId: 'worker',
              label: 'Worker',
              reviewEvidenceId: evidenceId,
              result: JSON.stringify({ checksRun: ['all tests pass'], passed: true }),
            },
          ]),
        }}
      />
    </HtmlCardHostProvider>,
  );
}
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
it('click expands the production Fleet card and only sends owning session/id/file selectors', async () => {
  const read = vi.fn(async () => ({ ok: true as const, evidence }));
  window.electronAPI = {
    fleetReviewRead: read,
    fleetReviewForget: vi.fn(async () => ({ ok: true })),
  } as unknown as typeof window.electronAPI;
  mount();
  expect(read).not.toHaveBeenCalled();
  expect(screen.getByText(/Worker-reported/)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Review changes' }));
  await waitFor(() => expect(screen.getByText('M · changed.ts')).toBeTruthy());
  expect(read).toHaveBeenCalledWith({
    evidenceId: id,
    ownerSessionId: 'manager',
    workerSessionId: 'worker',
  });
  fireEvent.click(screen.getByText('M · changed.ts'));
  await waitFor(() =>
    expect(read).toHaveBeenLastCalledWith({
      evidenceId: id,
      ownerSessionId: 'manager',
      workerSessionId: 'worker',
      file: 'changed.ts',
    }),
  );
  fireEvent.click(screen.getByRole('button', { name: 'Forget review data' }));
  await waitFor(() => expect(screen.getByText('Review data forgotten.')).toBeTruthy());
  expect(screen.queryByTestId('fleet-inline-review')).toBeNull();
});
it('shows backend and missing metadata states without a live git fallback', () => {
  window.electronAPI = {} as typeof window.electronAPI;
  const view = mount();
  expect(screen.getByText(/Remote\/headless review is not supported/)).toBeTruthy();
  view.unmount();
  window.electronAPI = { fleetReviewRead: vi.fn() } as unknown as typeof window.electronAPI;
  mount('');
  expect(screen.getByText(/No host-captured range/)).toBeTruthy();
});
it('surfaces revoked/wrong-owner responses and dirty states truthfully', async () => {
  const read = vi
    .fn()
    .mockResolvedValueOnce({ ok: false, error: 'Review revoked' })
    .mockResolvedValue({
      ok: true,
      evidence: {
        ...evidence,
        availability: 'dirty',
        files: [],
        reason: 'Uncommitted work retained in worktree',
      },
    });
  window.electronAPI = { fleetReviewRead: read } as unknown as typeof window.electronAPI;
  const view = mount(id, 'wrong-owner');
  fireEvent.click(screen.getByRole('button', { name: 'Review changes' }));
  await waitFor(() => expect(screen.getByRole('alert').textContent).toBe('Review revoked'));
  expect(read.mock.calls[0][0].ownerSessionId).toBe('wrong-owner');
  view.unmount();
  mount();
  fireEvent.click(screen.getByRole('button', { name: 'Review changes' }));
  await waitFor(() =>
    expect(screen.getByText('Uncommitted work retained in worktree')).toBeTruthy(),
  );
  expect(screen.queryByLabelText('Changed files')).toBeNull();
});
