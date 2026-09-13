import React from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentWorkspaces from '../../src/components/IntentWorkspaces';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';

const workspace: IntentWorkspace = {
  id: 'work-1',
  projectRoot: '/project',
  title: 'Export results',
  outcome: 'Filtered exports',
  constraints: '',
  successCriteria: '',
  sourceUrl: '',
  status: 'draft',
  revision: 1,
  createdAt: '2026-09-12T12:00:00Z',
  updatedAt: '2026-09-12T12:00:00Z',
};
const api = vi.fn();
beforeEach(() => {
  api.mockReset();
  Object.assign(window.electronAPI, { intentWorkspaceRequest: api });
  api.mockImplementation(async (request) =>
    request.action === 'history'
      ? { action: 'history', revisions: [] }
      : { action: 'list', workspaces: [] },
  );
});

describe('intent workspaces', () => {
  it('opens existing work on Overview and supports roving keyboard tab navigation', async () => {
    api.mockImplementation(async (input) => {
      if (input.action === 'list') return { action: 'list', workspaces: [workspace] };
      const values: Record<string, unknown> = {
        history: { revisions: [] },
        executions: { executions: [], links: [] },
        evidence: { criteria: [], evidence: [], reviews: [] },
        directions: { directions: [] },
        sources: { sources: [], comments: [] },
        artifacts: {
          artifacts: [],
          annotations: [],
          demonstrations: [],
          groups: [],
          selections: [],
        },
        controls: { controls: [] },
      };
      return { action: input.action, ...(values[input.action] as object) };
    });
    render(<IntentWorkspaces onClose={vi.fn()} execution={{ sessions: [] }} />);
    const overview = await screen.findByRole('tab', { name: 'Overview' });
    expect(overview).toHaveAttribute('aria-selected', 'true');
    expect(overview).toHaveAttribute('tabindex', '0');
    fireEvent.keyDown(overview, { key: 'ArrowRight' });
    const intent = screen.getByRole('tab', { name: 'Intent' });
    expect(intent).toHaveAttribute('aria-selected', 'true');
    expect(intent).toHaveFocus();
    expect(screen.getByRole('tabpanel')).toHaveAttribute('aria-labelledby', intent.id);
    fireEvent.keyDown(intent, { key: 'End' });
    expect(screen.getByRole('tab', { name: 'History' })).toHaveFocus();
    fireEvent.keyDown(screen.getByRole('tab', { name: 'History' }), { key: 'Home' });
    expect(overview).toHaveFocus();
  });

  it('filters the work list without losing the selected intent or its draft', async () => {
    api.mockImplementation(async (input) =>
      input.action === 'list'
        ? {
            action: 'list',
            workspaces: [workspace, { ...workspace, id: 'other', title: 'Login fix' }],
          }
        : { action: 'history', revisions: [] },
    );
    render(<IntentWorkspaces onClose={vi.fn()} />);
    await screen.findByDisplayValue('Export results');
    fireEvent.change(screen.getByLabelText('Constraints'), {
      target: { value: 'Keep permissions' },
    });
    fireEvent.change(screen.getByRole('searchbox', { name: 'Find work' }), {
      target: { value: 'Login' },
    });
    expect(
      screen.queryByRole('button', { name: 'Export results · Unsaved draft' }),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText('Constraints')).toHaveValue('Keep permissions');
    fireEvent.change(screen.getByRole('searchbox', { name: 'Find work' }), {
      target: { value: '' },
    });
    expect(
      screen.getByRole('button', { name: 'Export results · Unsaved draft' }),
    ).toBeInTheDocument();
  });

  it('clears the dirty marker when edits are reverted to the saved values', async () => {
    api.mockImplementation(async (input) =>
      input.action === 'list'
        ? { action: 'list', workspaces: [workspace] }
        : { action: 'history', revisions: [] },
    );
    render(<IntentWorkspaces onClose={vi.fn()} />);
    await screen.findByDisplayValue('Export results');
    fireEvent.change(screen.getByLabelText('Constraints'), { target: { value: 'Temporary' } });
    fireEvent.change(screen.getByLabelText('Constraints'), { target: { value: '' } });
    expect(screen.getByRole('button', { name: 'Save revision' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Export results draft' })).toBeInTheDocument();
  });
  it('creates a project-owned intent and exposes the saved revision', async () => {
    render(<IntentWorkspaces defaultRoot="/project" onClose={vi.fn()} />);
    fireEvent.click(await screen.findByText('Create an intent'));
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Export results' } });
    fireEvent.change(screen.getByLabelText('Desired outcome'), {
      target: { value: 'Filtered exports' },
    });
    api.mockImplementation(async (request) =>
      request.action === 'create'
        ? { action: 'create', workspace }
        : { action: 'history', revisions: [] },
    );
    fireEvent.click(screen.getByRole('button', { name: 'Create workspace' }));
    expect(await screen.findByText('Saved revision 1.')).toBeInTheDocument();
    expect(api).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'create',
        projectRoot: '/project',
        fields: expect.objectContaining({ title: 'Export results', outcome: 'Filtered exports' }),
      }),
    );
    expect(screen.getByText('Intent history')).toBeInTheDocument();
  });

  it('keeps the draft and expected revision after a rejected save', async () => {
    api.mockImplementation(async (request) => {
      if (request.action === 'list') return { action: 'list', workspaces: [workspace] };
      if (request.action === 'history') return { action: 'history', revisions: [] };
      throw new Error('This workspace changed elsewhere');
    });
    render(<IntentWorkspaces onClose={vi.fn()} />);
    await screen.findByDisplayValue('Export results');
    fireEvent.change(screen.getByLabelText('Constraints'), { target: { value: 'CSV only' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save revision' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('changed elsewhere');
    expect(screen.getByLabelText('Constraints')).toHaveValue('CSV only');
    expect(api).toHaveBeenCalledWith(
      expect.objectContaining({ action: 'update', expectedRevision: 1 }),
    );
    expect(screen.getByRole('button', { name: 'Save revision' })).not.toBeDisabled();
  });

  it('retains separate unsaved drafts while navigating between workspaces', async () => {
    api.mockImplementation(async (request) =>
      request.action === 'list'
        ? {
            action: 'list',
            workspaces: [workspace, { ...workspace, id: 'work-2', title: 'Login fix' }],
          }
        : { action: 'history', revisions: [] },
    );
    render(<IntentWorkspaces onClose={vi.fn()} />);
    await screen.findByDisplayValue('Export results');
    fireEvent.change(screen.getByLabelText('Constraints'), { target: { value: 'CSV only' } });
    fireEvent.click(screen.getByRole('button', { name: 'Login fix draft' }));
    expect(screen.getByLabelText('Constraints')).toHaveValue('');
    fireEvent.click(screen.getByRole('button', { name: 'Export results · Unsaved draft' }));
    expect(screen.getByLabelText('Constraints')).toHaveValue('CSV only');
  });

  it('reports an unavailable host without showing an empty success', async () => {
    api.mockRejectedValue(new Error('Host unavailable'));
    render(<IntentWorkspaces onClose={vi.fn()} />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Host unavailable');
    await waitFor(() => expect(screen.queryByText('Loading work…')).not.toBeInTheDocument());
  });
});
