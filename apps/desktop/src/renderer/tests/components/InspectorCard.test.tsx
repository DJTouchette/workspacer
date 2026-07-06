import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';
import React from 'react';
import { InspectorCard } from '../../src/components/claude/InspectorCard';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';

/**
 * The shared InspectorCard renders all five sections (Plan, Flows, Agents,
 * Files, Usage) purely from the snapshot prop, and degrades each section to its
 * own empty state when the snapshot lacks that data. It must also survive an
 * absent snapshot (the hover peek can render before one arrives).
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

const richSnapshot = () =>
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
    fileChanges: [{ path: '/repo/src/App.tsx', toolName: 'Edit', input: {}, timestamp: 1 }],
    subagents: [{ id: 'sub-1', type: 'explorer', status: 'running', startedAt: 1 }],
    workflows: [
      {
        runId: 'run-1',
        name: 'review',
        status: 'running',
        startedAt: 1,
        phases: [],
        agents: [],
      },
    ],
    usage: {
      model: 'claude-opus-4-8',
      contextTokens: 50000,
      contextLimit: 200000,
      totalInputTokens: 1200,
      totalOutputTokens: 800,
      costUSD: 0.42,
    },
    totalToolCalls: 7,
  });

describe('InspectorCard', () => {
  it('surfaces every section tab with its badge from a full snapshot', () => {
    render(<InspectorCard snapshot={richSnapshot()} />);
    // Plan badge = done/total.
    expect(
      within(screen.getByRole('button', { name: /Plan/ })).getByText('1/3'),
    ).toBeInTheDocument();
    // Files / Flows / Agents badges are the counts.
    expect(
      within(screen.getByRole('button', { name: /Files/ })).getByText('1'),
    ).toBeInTheDocument();
    expect(
      within(screen.getByRole('button', { name: /Flows/ })).getByText('1'),
    ).toBeInTheDocument();
    expect(
      within(screen.getByRole('button', { name: /Agents/ })).getByText('1'),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Usage/ })).toBeInTheDocument();
  });

  it('renders each tab body from the snapshot', () => {
    render(<InspectorCard snapshot={richSnapshot()} />);
    fireEvent.click(screen.getByRole('button', { name: /Plan/ }));
    expect(screen.getByText('Wire the inspector')).toBeInTheDocument();
    expect(screen.getByText('Wiring the inspector')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Files/ }));
    expect(screen.getByText('App.tsx')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Usage/ }));
    expect(screen.getByText('claude-opus-4-8')).toBeInTheDocument();
    expect(screen.getByText('Cost')).toBeInTheDocument();
  });

  it('degrades each section to an empty state when its data is missing', () => {
    render(<InspectorCard snapshot={makeSnapshot()} />);
    // Files is the default tab for an idle snapshot.
    expect(screen.getByText('No files changed yet')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Plan/ }));
    expect(screen.getByText('No plan yet')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Flows/ }));
    expect(screen.getByText('No workflows running')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Agents/ }));
    expect(screen.getByText('No subagents yet')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Usage/ }));
    expect(screen.getByText('No usage data yet')).toBeInTheDocument();
  });

  it('opens on the forced initialTab and shows the agent name header', () => {
    render(
      <InspectorCard snapshot={richSnapshot()} agentName="Refactor agent" initialTab="plan" />,
    );
    // Plan body is visible without a click because initialTab pinned it.
    expect(screen.getByText('1/3 done')).toBeInTheDocument();
    expect(screen.getByText('Refactor agent')).toBeInTheDocument();
  });

  it('renders without crashing when the snapshot is absent', () => {
    render(<InspectorCard snapshot={undefined} />);
    expect(screen.getByText('No files changed yet')).toBeInTheDocument();
  });
});
