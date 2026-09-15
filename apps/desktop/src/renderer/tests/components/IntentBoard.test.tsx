import React from 'react';
import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import IntentBoard from '../../src/components/IntentBoard';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';

const draft: IntentWorkspace = {
  id: 'draft',
  title: 'Export results',
  outcome: 'Download filtered results',
  projectRoot: '/project',
  constraints: '',
  successCriteria: '',
  sourceUrl: '',
  status: 'draft',
  revision: 1,
  createdAt: '',
  updatedAt: '',
};
const review: IntentWorkspace = { ...draft, id: 'review', title: 'Fix login', status: 'review' };
const active: IntentWorkspace = {
  ...draft,
  id: 'active',
  title: 'Improve search',
  status: 'active',
};
function setup() {
  const onOpen = vi.fn();
  render(
    <IntentBoard
      groups={[{ id: 'project', label: 'My project', items: [draft, review, active] }]}
      selected={null}
      drafts={{}}
      executionIndex={{
        active: [
          { sessionId: 'agent', hub: '', label: 'Agent', provider: 'claude', cwd: '/project' },
        ],
      }}
      sessions={[
        { sessionId: 'agent', status: 'running', pendingQuestions: [{ question: 'Which index?' }] },
      ]}
      disabled={false}
      loading={false}
      onOpen={onOpen}
      onCreate={vi.fn()}
    />,
  );
  return { onOpen };
}
describe('intent board', () => {
  it('groups work by lifecycle and includes both review and live questions in Needs me', () => {
    setup();
    expect(
      within(screen.getByRole('region', { name: 'Draft' })).getByRole('button', {
        name: 'Export results draft',
      }),
    ).toBeInTheDocument();
    expect(
      within(screen.getByRole('region', { name: 'Needs review' })).getByRole('button', {
        name: 'Fix login review',
      }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Needs me 2' }));
    expect(screen.queryByRole('button', { name: 'Export results draft' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Fix login review' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Improve search active' })).toBeInTheDocument();
  });
  it('opens existing start and review workflows from card actions', () => {
    const { onOpen } = setup();
    fireEvent.click(screen.getByRole('button', { name: 'Start Export results' }));
    expect(onOpen).toHaveBeenLastCalledWith('draft', 'overview');
    fireEvent.click(screen.getByRole('button', { name: 'Review Fix login' }));
    expect(onOpen).toHaveBeenLastCalledWith('review', 'review');
  });
  it('routes a drop into Complete through review without moving the card optimistically', () => {
    const { onOpen } = setup();
    const card = screen
      .getByRole('button', { name: 'Improve search active' })
      .closest('[draggable]')!;
    const dataTransfer = { setData: vi.fn(), effectAllowed: '', dropEffect: '' };
    fireEvent.dragStart(card, { dataTransfer });
    fireEvent.drop(screen.getByRole('region', { name: 'Complete' }), { dataTransfer });
    expect(onOpen).toHaveBeenCalledWith('active', 'review');
    expect(
      within(screen.getByRole('region', { name: 'Active' })).getByRole('button', {
        name: 'Improve search active',
      }),
    ).toBeInTheDocument();
  });
});
