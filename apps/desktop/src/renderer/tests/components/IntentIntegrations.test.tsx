import React from 'react';
import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentIntegrations from '../../src/components/IntentIntegrations';
import type { IntentIntegrationView } from '../../../main/shared/intentIntegrations';
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
  status: 'draft',
  createdAt: '',
  updatedAt: '',
};
const jira: IntentIntegrationView = {
  id: 'j',
  projectRoot: '/project',
  version: 1,
  deleted: false,
  references: 1,
  enabled: true,
  name: 'Team Jira',
  provider: 'jira',
  baseUrl: 'https://team.atlassian.net',
  credentialEnv: 'WORKSPACER_SOURCE_TEAM',
  repository: '',
  defaultProjectKey: 'TEAM',
};
let connections: IntentIntegrationView[];
const request = vi.fn();
beforeEach(() => {
  connections = [
    jira,
    {
      ...jira,
      id: 'a',
      provider: 'ado',
      name: 'Azure',
      baseUrl: 'https://dev.azure.com/org/project',
      references: 0,
    },
  ];
  request.mockReset().mockImplementation(async (input) => {
    if (input.action === 'integrations') return { action: input.action, integrations: connections };
    if (input.action === 'saveIntegration') {
      connections = connections
        .filter((c) => c.id !== input.integrationId)
        .concat({
          ...input.integration,
          id: input.integrationId,
          version: input.expectedVersion + 1,
          references: 0,
        });
    }
    return { action: input.action };
  });
  Object.assign(window.electronAPI, { intentWorkspaceRequest: request });
});
async function open() {
  const attached = vi.fn();
  const result = render(
    <IntentIntegrations workspace={workspace} disabled={false} onAttached={attached} />,
  );
  const details = screen.getByText('Project tracker integrations').closest('details')!;
  details.open = true;
  fireEvent(details, new Event('toggle'));
  await waitFor(() => expect(screen.getByLabelText('Named integration')).toBeEnabled());
  return { ...result, attached };
}
it('previews normalized identifiers and submits an immutable versioned reference through an accessible form', async () => {
  const { attached } = await open();
  fireEvent.change(screen.getByLabelText('Named integration'), { target: { value: 'j' } });
  fireEvent.change(screen.getByLabelText('Native identifier'), { target: { value: 'team-123' } });
  expect(screen.getByLabelText('Canonical source URL')).toHaveTextContent(
    'https://team.atlassian.net/browse/TEAM-123',
  );
  fireEvent.submit(screen.getByRole('form', { name: 'Attach project source' }));
  await waitFor(() => expect(attached).toHaveBeenCalledOnce());
  expect(request).toHaveBeenCalledWith(
    expect.objectContaining({
      action: 'attachSource',
      id: 'w',
      expectedRevision: 1,
      sourceId: expect.any(String),
      reference: {
        integrationId: 'j',
        expectedIntegrationVersion: 1,
        objectType: 'issue',
        identifier: 'team-123',
        repository: '',
      },
    }),
  );
});
it('requires explicit PR repository and prevents removing referenced connections', async () => {
  await open();
  expect(screen.getByRole('button', { name: 'Remove Team Jira' })).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Remove Azure' })).toBeEnabled();
  fireEvent.change(screen.getByLabelText('Named integration'), { target: { value: 'a' } });
  fireEvent.change(screen.getByLabelText('Object type'), { target: { value: 'pull-request' } });
  fireEvent.change(screen.getByLabelText('Native identifier'), { target: { value: '12' } });
  expect(screen.getByRole('button', { name: 'Attach source' })).toBeDisabled();
  expect(screen.getByText(/A repository is required/)).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText(/PR repository/), { target: { value: 'repo' } });
  expect(screen.getByLabelText('Canonical source URL')).toHaveTextContent(
    '/_git/repo/pullrequest/12',
  );
  expect(screen.getByRole('button', { name: 'Attach source' })).toBeEnabled();
});
it('preserves the same source ID after a lost response and renders host conflicts', async () => {
  await open();
  fireEvent.change(screen.getByLabelText('Named integration'), { target: { value: 'j' } });
  fireEvent.change(screen.getByLabelText('Native identifier'), { target: { value: 'TEAM-1' } });
  const previous = request.getMockImplementation()!;
  request.mockImplementation((input) =>
    input.action === 'attachSource'
      ? Promise.reject(new Error('Integration changed. Reload and preview again.'))
      : previous(input),
  );
  for (let i = 0; i < 2; i++) {
    fireEvent.submit(screen.getByRole('form', { name: 'Attach project source' }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Attach source' })).toBeEnabled(),
    );
  }
  expect(screen.getByRole('alert')).toHaveTextContent('Integration changed');
  const calls = request.mock.calls.filter(([r]) => r.action === 'attachSource');
  expect(calls).toHaveLength(2);
  expect(calls[0][0]).toEqual(calls[1][0]);
});
it('creates, edits and disables shared metadata without supplying a secret', async () => {
  await open();
  fireEvent.change(screen.getByLabelText('Connection name'), { target: { value: 'Another Jira' } });
  fireEvent.change(screen.getByLabelText('Jira site base URL'), {
    target: { value: 'https://another.atlassian.net' },
  });
  fireEvent.change(screen.getByLabelText('Connection credential environment name'), {
    target: { value: 'WORKSPACER_SOURCE_ANOTHER' },
  });
  fireEvent.submit(screen.getByRole('form', { name: 'Project connection editor' }));
  await screen.findByRole('button', { name: 'Edit Another Jira' });
  const save = request.mock.calls.find(([r]) => r.action === 'saveIntegration')![0];
  expect(save).toMatchObject({
    expectedVersion: 0,
    operationId: expect.any(String),
    integration: { credentialEnv: 'WORKSPACER_SOURCE_ANOTHER' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Edit Another Jira' }));
  expect(screen.getByLabelText('Connection name')).toHaveValue('Another Jira');
  fireEvent.click(screen.getByRole('button', { name: 'Cancel edit' }));
  fireEvent.click(screen.getByRole('button', { name: 'Disable Another Jira' }));
  await screen.findByRole('button', { name: 'Enable Another Jira' });
  expect(screen.queryByRole('option', { name: 'Another Jira' })).not.toBeInTheDocument();
});
it('does not enable attachments when the host rejects loading integrations', async () => {
  request.mockRejectedValue(new Error('Host offline'));
  render(<IntentIntegrations workspace={workspace} disabled={false} onAttached={vi.fn()} />);
  const details = screen.getByText('Project tracker integrations').closest('details')!;
  details.open = true;
  fireEvent(details, new Event('toggle'));
  expect(await screen.findByRole('alert')).toHaveTextContent('Host offline');
  expect(screen.getByRole('button', { name: 'Attach source' })).toBeDisabled();
});
