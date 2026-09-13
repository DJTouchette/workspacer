import React, { useState } from 'react';
import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentProjects, { type IntentProjectsDraft } from '../../src/components/IntentProjects';
import type { IntentProject } from '../../../main/shared/intentProject';
const request = vi.fn(),
  changed = vi.fn();
let projects: IntentProject[];
beforeEach(() => {
  changed.mockReset();
  projects = [
    {
      id: 'project',
      name: 'Product',
      revision: 1,
      createdAt: '',
      updatedAt: '',
      repositories: [
        {
          id: 'repo',
          projectId: 'project',
          root: '/old/root',
          revision: 1,
          createdAt: '',
          updatedAt: '',
        },
      ],
    },
  ];
  request.mockReset().mockImplementation(async (input) => {
    if (input.action === 'projects')
      return { action: 'projects', projects: structuredClone(projects) };
    if (input.action === 'renameIntentProject') {
      projects[0].name = input.name;
      projects[0].revision++;
      return { action: input.action, project: projects[0] };
    }
    if (input.action === 'relocateIntentRepository') {
      projects[0].repositories[0].root = input.root;
      projects[0].revision++;
      return { action: input.action, project: projects[0], workspaces: [] };
    }
    throw new Error('Unexpected request');
  });
  Object.assign(window.electronAPI, { intentWorkspaceRequest: request, saveConfig: vi.fn() });
});
function View({ visible = true } = {}) {
  const [draft, setDraft] = useState<IntentProjectsDraft>({
    projectId: '',
    action: 'renameIntentProject',
    repositoryId: '',
    name: '',
    root: '',
    reason: '',
  });
  return (
    <IntentProjects visible={visible} draft={draft} onDraftChange={setDraft} onChanged={changed} />
  );
}
it('shows stable project/repository identity and saves an explicitly edited project name', async () => {
  render(<View />);
  fireEvent.click(await screen.findByRole('button', { name: 'Manage Product' }));
  expect(screen.getByText('Repository repo')).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('Project name'), { target: { value: 'Whole product' } });
  expect(request.mock.calls.filter(([input]) => input.action !== 'projects')).toHaveLength(0);
  fireEvent.click(screen.getByRole('button', { name: 'Save project name' }));
  expect(await screen.findByRole('button', { name: 'Manage Whole product' })).toBeInTheDocument();
  expect(request).toHaveBeenCalledWith({
    action: 'renameIntentProject',
    projectId: 'project',
    expectedRevision: 1,
    name: 'Whole product',
  });
  expect(changed).toHaveBeenCalledTimes(1);
  expect(window.electronAPI.saveConfig).not.toHaveBeenCalled();
});
it('reviews old and new repository roots and records an explicit relocation reason', async () => {
  render(<View />);
  fireEvent.click(await screen.findByRole('button', { name: 'Manage Product' }));
  fireEvent.change(screen.getByLabelText('Project change'), {
    target: { value: 'relocateIntentRepository' },
  });
  fireEvent.change(screen.getByLabelText('Repository to relocate'), { target: { value: 'repo' } });
  fireEvent.change(screen.getByLabelText('New repository root'), {
    target: { value: '/new/root' },
  });
  fireEvent.change(screen.getByLabelText('Reason for relocation'), {
    target: { value: 'Checkout moved' },
  });
  expect(
    screen.getByText(/All work items in this repository receive a new intent revision/),
  ).toBeInTheDocument();
  expect(
    request.mock.calls.filter(([input]) => input.action === 'relocateIntentRepository'),
  ).toHaveLength(0);
  fireEvent.click(screen.getByRole('button', { name: 'Save root relocation' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith({
      action: 'relocateIntentRepository',
      projectId: 'project',
      repositoryId: 'repo',
      expectedRevision: 1,
      root: '/new/root',
      reason: 'Checkout moved',
    }),
  );
  expect(await screen.findByText('/new/root')).toBeInTheDocument();
});
it('preserves failed drafts across navigation and refuses overwriting a newer project revision', async () => {
  const { rerender } = render(<View />);
  fireEvent.click(await screen.findByRole('button', { name: 'Manage Product' }));
  fireEvent.change(screen.getByLabelText('Project name'), { target: { value: 'My draft name' } });
  const normal = request.getMockImplementation()!;
  request.mockImplementation(async (input) => {
    if (input.action === 'renameIntentProject') {
      projects[0].revision = 2;
      throw new Error('Project changed elsewhere');
    }
    return normal(input);
  });
  fireEvent.click(screen.getByRole('button', { name: 'Save project name' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Project changed elsewhere');
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Save project name' })).toBeDisabled(),
  );
  rerender(<View visible={false} />);
  rerender(<View />);
  expect(screen.getByLabelText('Project name')).toHaveValue('My draft name');
  expect(screen.getByRole('button', { name: 'Save project name' })).toBeDisabled();
  expect(
    request.mock.calls.filter(([input]) => input.action === 'renameIntentProject'),
  ).toHaveLength(1);
  fireEvent.click(
    screen.getByRole('button', { name: 'Use latest project revision for this draft' }),
  );
  expect(screen.getByLabelText('Project name')).toHaveValue('My draft name');
  expect(screen.getByRole('button', { name: 'Save project name' })).toBeEnabled();
  expect(
    request.mock.calls.filter(([input]) => input.action === 'renameIntentProject'),
  ).toHaveLength(1);
});
