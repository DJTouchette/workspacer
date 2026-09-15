import React from 'react';
import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentJiraImport from '../../src/components/IntentJiraImport';
const api = vi.fn();
beforeEach(() => {
  api.mockReset();
  Object.assign(window.electronAPI, { intentWorkspaceRequest: api });
});
it('imports through a configured connection and reuses the operation ID after a lost response', async () => {
  const workspace = { id: 'created', title: 'Export CSV', status: 'draft' };
  let attempts = 0;
  api.mockImplementation(async (input) => {
    if (input.action === 'jiraIntegrations')
      return {
        action: input.action,
        integrations: [{ id: 'jira', name: 'Team Jira', version: 3 }],
      };
    if (++attempts === 1) throw new Error('Response lost');
    return { action: 'importJiraIntent', workspace };
  });
  const onImported = vi.fn();
  const onBusyChange = vi.fn();
  render(
    <IntentJiraImport
      visible
      defaultRoot="/project"
      roots={['/project']}
      onImported={onImported}
      onBusyChange={onBusyChange}
    />,
  );
  await screen.findByRole('option', { name: 'Team Jira' });
  fireEvent.change(screen.getByLabelText('Jira issue key or URL'), {
    target: { value: 'TEAM-123' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Import as draft' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Response lost');
  fireEvent.click(screen.getByRole('button', { name: 'Import as draft' }));
  await waitFor(() => expect(onImported).toHaveBeenCalledWith(workspace));
  const calls = api.mock.calls
    .map(([input]) => input)
    .filter((input) => input.action === 'importJiraIntent');
  expect(calls).toHaveLength(2);
  expect(calls[1]).toEqual(calls[0]);
  expect(calls[0]).toMatchObject({
    projectRoot: '/project',
    identifier: 'TEAM-123',
    integrationId: 'jira',
    expectedIntegrationVersion: 3,
  });
  expect(onBusyChange).toHaveBeenLastCalledWith(false);
});
it('explains missing setup and disables import when no Jira connection exists', async () => {
  api.mockResolvedValue({ action: 'jiraIntegrations', integrations: [] });
  render(
    <IntentJiraImport
      visible
      defaultRoot="/project"
      roots={[]}
      onImported={vi.fn()}
      onBusyChange={vi.fn()}
    />,
  );
  expect(await screen.findByText(/Configure a Jira connection/)).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Import as draft' })).toBeDisabled();
});
