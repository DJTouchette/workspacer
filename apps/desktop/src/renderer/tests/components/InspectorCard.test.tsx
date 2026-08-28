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

  it('opens Codex subagent rows while keeping the aggregate monitor hidden', () => {
    const opened: unknown[] = [];
    const handler = (event: Event) => opened.push((event as CustomEvent).detail);
    window.addEventListener('agentwatch:open', handler);
    try {
      render(
        <InspectorCard
          snapshot={makeSnapshot({
            provider: 'codex',
            ambientState: 'background',
            subagents: [
              {
                id: 'child-1',
                type: 'codex',
                status: 'running',
                startedAt: 1,
                description: 'inspect this',
              },
            ],
          })}
          initialTab="agents"
        />,
      );

      expect(screen.queryByRole('button', { name: /Monitor/ })).not.toBeInTheDocument();
      fireEvent.click(screen.getByTitle('Watch this agent in a pane'));
      expect(opened).toEqual([
        {
          sessionId: 'sess-1',
          kind: 'subagent',
          id: 'child-1',
          title: 'Agent: codex',
        },
      ]);
    } finally {
      window.removeEventListener('agentwatch:open', handler);
    }
  });

  // ── the tabs a harness cannot fill ──────────────────────────────────────

  it('hides Plan, Flows and Agents on a harness that has none of them', () => {
    // pi ships bash/edit/find/grep/ls/powershell/read/write and nothing else —
    // no todo tool, no task tool. "No plan yet" on a pi session reads as "not
    // yet" when the truth is "never", and nothing anywhere would say so.
    render(<InspectorCard snapshot={makeSnapshot({ provider: 'pi' })} />);
    expect(screen.queryByRole('button', { name: /Plan/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Flows/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Agents/ })).not.toBeInTheDocument();
    // Files and Usage are universal and stay.
    expect(screen.getByRole('button', { name: /Files/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Usage/ })).toBeInTheDocument();
    expect(screen.getByText('No files changed yet')).toBeInTheDocument();
  });

  it('keeps Plan and Agents for a managed harness that has them', () => {
    // copilot: a `todos` table in its own session db, and `subagent.*` frames.
    // Only Flows is dropped — workflow runs are Claude Code artifacts.
    render(<InspectorCard snapshot={makeSnapshot({ provider: 'copilot' })} />);
    expect(screen.getByRole('button', { name: /Plan/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Agents/ })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Flows/ })).not.toBeInTheDocument();
  });

  it('shows a section anyway when data for it actually arrived', () => {
    // The caps table is a belief; a row that arrived is proof. If a provider
    // ever reports something we did not expect, the failure must be a visible
    // tab and a stale comment — never dropped data.
    render(
      <InspectorCard
        snapshot={makeSnapshot({
          provider: 'pi',
          plan: { steps: [{ content: 'Surprise', status: 'pending' }], updatedAt: 1 },
        })}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Plan/ }));
    expect(screen.getByText('Surprise')).toBeInTheDocument();
  });

  it('falls back to Files when initialTab names a section this harness lacks', () => {
    // The rail and the Fleet Deck both pass an initialTab, and a card can be
    // handed a different provider's snapshot without remounting. Neither may
    // leave the strip with nothing selected and a blank body.
    render(<InspectorCard snapshot={makeSnapshot({ provider: 'pi' })} initialTab="plan" />);
    expect(screen.getByText('No files changed yet')).toBeInTheDocument();
  });

  it('does not offer to open a copilot subagent row it cannot show', () => {
    // The row is real — copilot reports the child's whole lifecycle. What does
    // not exist is a transcript: the child's frames are dropped so they cannot
    // leak into the parent's conversation, and nothing persists them. A click
    // target here would land on "Transcript unavailable".
    render(
      <InspectorCard
        snapshot={makeSnapshot({
          provider: 'copilot',
          subagents: [{ id: 'agent-1', type: 'explore', status: 'running', startedAt: 1 }],
        })}
        initialTab="agents"
      />,
    );
    expect(screen.getByText('explore')).toBeInTheDocument();
    expect(screen.queryByTitle('Watch this agent in a pane')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Monitor/ })).not.toBeInTheDocument();
  });
});
