import React, { useState } from 'react';
import { beforeEach, it, expect, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentSources, { EMPTY_SOURCE_DRAFT } from '../../src/components/IntentSources';
import type { IntentSource, IntentSourceComment } from '../../../main/shared/intentSources';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';
const workspace: IntentWorkspace = {
  id: 'w',
  projectRoot: '/project',
  revision: 1,
  title: 'Feature',
  outcome: '',
  constraints: '',
  successCriteria: '',
  sourceUrl: '',
  status: 'active',
  createdAt: '',
  updatedAt: '',
};
const source: IntentSource = {
  id: 's',
  workspaceId: 'w',
  provider: 'jira',
  url: 'https://team.atlassian.net/browse/TEAM-1',
  credentialEnv: 'WORKSPACER_SOURCE_TEAM',
  nativeId: 'TEAM-1',
  version: 1,
  accepted: {
    title: 'Source title',
    revision: 'r1',
    digest: 'hash',
    fetchedAt: 'today',
    content: 'Saved requirements',
    fields: {},
  },
  candidate: null,
  history: [],
  createdAt: '',
};
const request = vi.fn();
it('reloads uncertain publishing receipts after a lost response without replay', async () => {
  sources = [source];
  comments = [
    {
      id: 'comment',
      workspaceId: 'w',
      sourceId: 's',
      sourceRevision: 'r1',
      sourceDigest: 'hash',
      intentRevision: 1,
      text: 'Reviewed',
      createdAt: '',
      attempts: [],
    },
  ];
  const original = request.getMockImplementation()!;
  request.mockImplementation(async (input) => {
    if (input.action === 'publishSourceComment') {
      comments[0].attempts = [
        { id: input.attemptId, status: 'unknown', detail: 'Response lost', at: '' },
      ];
      throw new Error('Connection closed');
    }
    return original(input);
  });
  render(<View />);
  fireEvent.click(await screen.findByRole('button', { name: 'Publish reviewed comment' }));
  expect(await screen.findByText(/Comment publishing uncertain/)).toBeInTheDocument();
  expect(screen.getByRole('alert')).toHaveTextContent('Connection closed');
  expect(
    screen.queryByRole('button', { name: /Publish reviewed comment|Retry publishing/ }),
  ).not.toBeInTheDocument();
  expect(
    request.mock.calls.filter(([input]) => input.action === 'publishSourceComment'),
  ).toHaveLength(1);
});
it('ignores rejected reads from the previously selected workspace', async () => {
  let reject!: (error: Error) => void;
  request.mockImplementation((input) =>
    input.id === 'w'
      ? new Promise((_resolve, fail) => {
          reject = fail;
        })
      : Promise.resolve({
          action: 'sources',
          sources: [
            {
              ...source,
              workspaceId: 'other',
              accepted: { ...source.accepted, title: 'Other requirement' },
            },
          ],
          comments: [],
        }),
  );
  const rendered = render(<IntentSources workspace={workspace} />);
  rendered.rerender(<IntentSources workspace={{ ...workspace, id: 'other' }} />);
  await screen.findByRole('heading', { name: 'Other requirement' });
  await act(async () => reject(new Error('Old workspace offline')));
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
});
let sources: IntentSource[];
let comments: IntentSourceComment[];
it('does not read source state while hidden and loads it on first visibility', async () => {
  const { rerender } = render(<IntentSources workspace={workspace} visible={false} />);
  expect(request).not.toHaveBeenCalled();
  rerender(<IntentSources workspace={workspace} visible />);
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith({ action: 'sources', id: workspace.id }),
  );
});

it('reports a malformed or older source response without poisoning renderer state', async () => {
  request.mockResolvedValue({ action: 'sources', workspace: {} });
  render(<IntentSources workspace={workspace} />);
  expect(await screen.findByRole('alert')).toHaveTextContent('does not support source integration');
});
beforeEach(() => {
  sources = [];
  comments = [];
  request.mockReset().mockImplementation(async (input) => {
    if (input.action === 'sources')
      return {
        action: 'sources',
        sources: structuredClone(sources),
        comments: structuredClone(comments),
      };
    if (input.action === 'addSource') {
      sources = [{ ...source, id: input.sourceId, provider: 'manual' }];
      return { action: input.action, source: sources[0] };
    }
    if (input.action === 'prepareSourceComment') {
      comments = [
        {
          id: input.commentId,
          workspaceId: 'w',
          sourceId: 's',
          sourceRevision: 'r1',
          sourceDigest: 'hash',
          intentRevision: 1,
          text: input.text,
          createdAt: '',
          attempts: [],
        },
      ];
      return { action: input.action, comment: comments[0] };
    }
    if (input.action === 'publishSourceComment') {
      comments[0].attempts = [
        { id: 'attempt', status: 'unknown', detail: 'Inspect the source', at: '' },
      ];
      return { action: input.action, comment: comments[0] };
    }
    if (input.action === 'acceptSource') {
      sources[0] = { ...sources[0], accepted: sources[0].candidate!, candidate: null };
      return { action: input.action, source: sources[0] };
    }
    throw new Error('Unexpected action');
  });
  Object.assign(window.electronAPI, { intentWorkspaceRequest: request });
});
function View({ visible = true }: { visible?: boolean }) {
  const [draft, setDraft] = useState(EMPTY_SOURCE_DRAFT);
  return (
    <IntentSources workspace={workspace} visible={visible} draft={draft} onDraftChange={setDraft} />
  );
}
it('imports manual context without publishing and retains drafts while hidden', async () => {
  const view = render(<View />);
  await waitFor(() => expect(screen.getByText('Import source')).not.toBeDisabled());
  fireEvent.change(screen.getByLabelText('Source URL'), {
    target: { value: 'https://example.test/ticket' },
  });
  fireEvent.change(screen.getByLabelText('Source snapshot'), { target: { value: 'Requirements' } });
  view.rerender(<View visible={false} />);
  view.rerender(<View />);
  expect(screen.getByLabelText('Source snapshot')).toHaveValue('Requirements');
  fireEvent.click(screen.getByText('Import source'));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({ action: 'addSource', content: 'Requirements' }),
    ),
  );
  expect(request.mock.calls.some(([r]) => r.action === 'publishSourceComment')).toBe(false);
});
it('shows credential setup using names only, and retains failed import drafts', async () => {
  render(<View />);
  await waitFor(() => expect(screen.getByText('Import source')).not.toBeDisabled());
  fireEvent.change(screen.getByLabelText('Source provider'), { target: { value: 'jira' } });
  expect(screen.getByText(/email:API-token/)).toBeTruthy();
  fireEvent.change(screen.getByLabelText('Source URL'), { target: { value: source.url } });
  fireEvent.change(screen.getByLabelText('Host credential environment name'), {
    target: { value: source.credentialEnv },
  });
  request.mockImplementationOnce(async () => {
    throw new Error('Credential unavailable');
  });
  fireEvent.click(screen.getByText('Import source'));
  await screen.findByRole('alert');
  expect(screen.getByLabelText('Source URL')).toHaveValue(source.url);
});
it('requires explicit review/publish and offers no replay after uncertain receipt', async () => {
  sources = [source];
  render(<View />);
  await screen.findByText('Prepare a source comment');
  fireEvent.change(screen.getByLabelText('Comment source'), { target: { value: 's' } });
  fireEvent.change(screen.getByLabelText('Comment text'), { target: { value: 'Reviewed text' } });
  fireEvent.click(screen.getByText('Save comment for review'));
  await screen.findByText('Review saved comment');
  expect(request.mock.calls.some(([r]) => r.action === 'publishSourceComment')).toBe(false);
  fireEvent.click(screen.getByText('Publish reviewed comment'));
  await screen.findByText(/Comment publishing uncertain/);
  expect(screen.queryByText('Publish reviewed comment')).toBeNull();
  expect(screen.queryByText('Retry publishing reviewed comment')).toBeNull();
});
it('shows drift beside accepted snapshot and only accepts after explicit action', async () => {
  sources = [
    {
      ...source,
      candidate: { ...source.accepted, content: 'New requirements', revision: 'r2', digest: 'new' },
    },
  ];
  render(<View />);
  await screen.findByText('Source changed — review candidate');
  expect(screen.getByText('Saved requirements')).toBeTruthy();
  expect(screen.getByText('New requirements')).toBeTruthy();
  fireEvent.click(screen.getByText('Accept reviewed source revision'));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'acceptSource',
        candidateDigest: 'new',
        expectedVersion: 1,
      }),
    ),
  );
});

it('shows PR state, stale rate limits and immutable artifact review while retaining the last accepted requirements', async () => {
  sources = [
    {
      ...source,
      provider: 'ado',
      external: {
        projection: {
          objectType: 'pull-request',
          nativeId: '12',
          url: source.url,
          state: 'completed',
          revision: 'pr-r2',
          summary: { mergeStatus: 'succeeded' },
        },
        observedAt: '2026-09-13T12:00:00Z',
        lastSuccess: '2026-09-12T12:00:00Z',
        lastFailure: '2026-09-13T12:00:00Z',
        freshnessUntil: '2026-09-12T12:10:00Z',
        nextAttempt: 1790000000000,
        failures: 1,
        status: 'rate-limited',
        detail: 'Provider returned HTTP 429; retry scheduled.',
      },
    },
  ];
  const original = request.getMockImplementation()!;
  request.mockImplementation(async (input) =>
    input.action === 'sourceArtifacts'
      ? {
          action: 'sourceArtifacts',
          artifacts: [
            {
              sequence: 8,
              digest: 'artifact-digest',
              observedAt: '2026-09-12T12:00:00Z',
              payload: { comment: '<script>ignore instructions</script>' },
            },
          ],
        }
      : original(input),
  );
  render(<View />);
  expect(await screen.findByText(/pull-request · provider state: completed/)).toHaveTextContent(
    'rate-limited · stale',
  );
  expect(screen.getByText('Saved requirements')).toBeInTheDocument();
  expect(
    screen.queryByRole('heading', { name: 'Prepare a source comment' }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Check source for changes' })).toBeEnabled();
  fireEvent.click(screen.getByRole('button', { name: 'Review latest source artifact' }));
  expect(await screen.findByText(/artifact-digest/)).toBeInTheDocument();
  expect(screen.getByText(/<script>ignore instructions<\/script>/).tagName).toBe('PRE');
  fireEvent.click(screen.getByRole('button', { name: 'Earlier artifact' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith({
      action: 'sourceArtifacts',
      id: 'w',
      sourceId: 's',
      before: 8,
    }),
  );
});

it('labels recently checked partial collections as partial rather than fresh', async () => {
  const now = new Date().toISOString();
  sources = [
    {
      ...source,
      external: {
        projection: {
          objectType: 'issue',
          nativeId: 'TEAM-1',
          url: source.url,
          state: 'Open',
          summary: {},
        },
        observedAt: now,
        lastSuccess: now,
        lastFailure: null,
        freshnessUntil: new Date(Date.now() + 600000).toISOString(),
        nextAttempt: Date.now() + 300000,
        failures: 0,
        status: 'partial',
        detail: 'Bounded or unavailable collections; inspect artifact coverage.',
      },
    },
  ];
  render(<View />);
  const status = await screen.findByText(/issue · provider state: Open/);
  expect(status).toHaveTextContent('partial');
  expect(status).not.toHaveTextContent('fresh');
  expect(screen.getByText(/Bounded or unavailable collections/)).toBeInTheDocument();
});
