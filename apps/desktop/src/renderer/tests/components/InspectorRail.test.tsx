import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';
import React from 'react';
import { InspectorRail } from '../../src/components/claude/InspectorRail';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';

/**
 * The InspectorRail Plan tab: a plan-bearing snapshot should surface a Plan tab
 * with a done/total badge, and its body should render the checklist — including
 * the in_progress step's activeForm ("doing now") line.
 */

function makeSnapshot(overrides: Partial<ClaudeSessionSnapshot> = {}): ClaudeSessionSnapshot {
  return {
    sessionId: 'sess-1',
    cwd: '/repo',
    ptyId: 'sess-1',
    status: 'active',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    pendingApproval: null,
    pendingQuestions: null,
    subagents: [],
    workflows: [],
    ambientState: 'streaming',
    lastActivity: Date.now(),
    totalToolCalls: 0,
    usage: null,
    ...overrides,
  } as ClaudeSessionSnapshot;
}

const planSnapshot = () =>
  makeSnapshot({
    plan: {
      steps: [
        { content: 'Add the types', status: 'completed' },
        {
          content: 'Wire the inspector',
          status: 'in_progress',
          activeForm: 'Wiring the inspector',
        },
        { content: 'Cover it with tests', status: 'pending' },
      ],
      updatedAt: 1,
    },
  });

describe('InspectorRail — Plan tab', () => {
  it('shows a Plan tab with a done/total badge', () => {
    render(<InspectorRail session={planSnapshot()} onClose={() => {}} />);
    const planTab = screen.getByRole('button', { name: /Plan/ });
    expect(within(planTab).getByText('1/3')).toBeInTheDocument();
  });

  it('renders each step and the in_progress activeForm line', () => {
    render(<InspectorRail session={planSnapshot()} onClose={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Plan/ }));
    expect(screen.getByText('Add the types')).toBeInTheDocument();
    expect(screen.getByText('Wire the inspector')).toBeInTheDocument();
    expect(screen.getByText('Cover it with tests')).toBeInTheDocument();
    // activeForm ("doing now") line for the in_progress step
    expect(screen.getByText('Wiring the inspector')).toBeInTheDocument();
    // progress header
    expect(screen.getByText('1/3 done')).toBeInTheDocument();
  });

  it('shows an empty state when there is no plan', () => {
    render(<InspectorRail session={makeSnapshot()} onClose={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Plan/ }));
    expect(screen.getByText('No plan yet')).toBeInTheDocument();
  });
});
