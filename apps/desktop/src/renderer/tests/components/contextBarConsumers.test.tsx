import React from 'react';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AgentCard } from '../../src/components/AgentCard';
import { SessionStatusBar } from '../../src/components/claude/SessionStatusBar';
import type { AgentWorkspace } from '../../src/types/pane';

vi.mock('../../src/contexts/AttentionContext', () => ({
  useAttention: () => ({ openAgent: () => {}, sendMessage: () => {}, feed: [] }),
}));
vi.mock('../../src/hooks/usePageVisible', () => ({ usePageVisible: () => true }));
vi.mock('../../src/hooks/useGitBranch', () => ({ useGitBranch: () => '' }));
vi.mock('../../src/components/AgentCardBody', () => ({ AgentCardBody: () => <div /> }));

afterEach(cleanup);

const agent = {
  id: 'worker',
  name: 'worker',
  cwd: '/repo',
  provider: 'codex',
  sessionId: 'session',
} as unknown as AgentWorkspace;

const snapshot = (confirmed: boolean) =>
  ({
    sessionId: 'session',
    cwd: '/repo',
    status: 'active',
    provider: 'codex',
    ambientState: 'idle',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    subagents: [],
    workflows: [],
    requestedSelection: { model: 'gpt-5.6-codex', contextWindow: 1_000_000 },
    resolvedContextWindow: 1_000_000,
    usage: {
      contextTokens: 129_200,
      contextLimit: 1_000_000,
      totalInputTokens: 74_000_000,
      totalOutputTokens: 1_000_000,
      costUSD: 4,
    },
    statusLine: confirmed
      ? {
          modelDisplay: 'gpt-5.6-codex',
          contextWindowSize: 258_400,
          totalInputTokens: 74_000_000,
          totalOutputTokens: 1_000_000,
        }
      : undefined,
  }) as any;

describe('both exact context-bar consumers use the shared runtime-confirmed selector', () => {
  it('AgentCard fleet row hides a provisional requested window, then renders active occupancy', () => {
    const view = render(<AgentCard agent={agent} snapshot={snapshot(false)} onOpen={() => {}} />);
    expect(screen.queryByTestId('agent-row-context-bar')).toBeNull();
    view.rerender(<AgentCard agent={agent} snapshot={snapshot(true)} onOpen={() => {}} />);
    expect(screen.getByTestId('agent-row-context-bar')).toHaveAccessibleName(/50%/);
    expect(screen.getByText(/129k · 50%/)).toBeInTheDocument();
  });

  it('SessionStatusBar hides provisional context and keeps 75M billed throughput off its percentage', () => {
    const view = render(<SessionStatusBar snapshot={snapshot(false)} />);
    expect(screen.queryByTestId('session-status-context-bar')).toBeNull();
    view.rerender(<SessionStatusBar snapshot={snapshot(true)} />);
    expect(screen.getByTestId('session-status-context-bar')).toHaveAccessibleName(/50%/);
    expect(screen.getByTestId('session-status-context-bar')).not.toHaveTextContent('75M');
  });
});
