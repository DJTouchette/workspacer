import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import GuidePane from '../../src/panes/GuidePane';
import { GUIDE_AGENT_NAME, GUIDE_PRESETS, buildGuideKickoff } from '../../src/lib/guide';
import type { AgentWorkspace } from '../../src/types/pane';

/**
 * The Guide pane: scripted (free) bubbles + preset chips up front, a REAL
 * agent spawn only on an explicit question, and reuse of an already-running
 * guide instead of spawning a second one.
 */

function agent(overrides: Partial<AgentWorkspace>): AgentWorkspace {
  return {
    id: 'a1',
    name: 'some-agent',
    cwd: '/tmp/x',
    tabs: [],
    activeTabId: '',
    ...overrides,
  };
}

describe('GuidePane', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows the scripted opening bubble, every preset chip, and the usage note', () => {
    render(<GuidePane agents={[]} spawnGuide={vi.fn()} onJumpToAgent={vi.fn()} />);

    expect(screen.getByText(/live agent with tools/)).toBeInTheDocument();
    for (const preset of GUIDE_PRESETS) {
      expect(screen.getByRole('button', { name: preset.label })).toBeInTheDocument();
    }
    // The honest fine print is always visible before anything runs.
    expect(screen.getByText(/consume usage/)).toBeInTheDocument();
  });

  it('spawns the guide with the preset prompt and jumps to it', async () => {
    const spawnGuide = vi.fn().mockResolvedValue('guide-agent-id');
    const onJumpToAgent = vi.fn();
    render(<GuidePane agents={[]} spawnGuide={spawnGuide} onJumpToAgent={onJumpToAgent} />);

    fireEvent.click(screen.getByRole('button', { name: GUIDE_PRESETS[0].label }));

    await waitFor(() => expect(onJumpToAgent).toHaveBeenCalledWith('guide-agent-id'));
    expect(spawnGuide).toHaveBeenCalledWith(GUIDE_PRESETS[0].prompt);
  });

  it('submits a typed question on Enter', async () => {
    const spawnGuide = vi.fn().mockResolvedValue('guide-agent-id');
    render(<GuidePane agents={[]} spawnGuide={spawnGuide} onJumpToAgent={vi.fn()} />);

    const input = screen.getByPlaceholderText(/Ask anything about Workspacer/);
    fireEvent.change(input, { target: { value: 'what is a pane?' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() => expect(spawnGuide).toHaveBeenCalledWith('what is a pane?'));
  });

  it('reuses a running guide: sends to its session instead of spawning again', async () => {
    const spawnGuide = vi.fn();
    const onJumpToAgent = vi.fn();
    const guide = agent({ id: 'g1', name: GUIDE_AGENT_NAME, sessionId: 'sess-guide' });
    render(
      <GuidePane
        agents={[agent({ id: 'a1' }), guide]}
        spawnGuide={spawnGuide}
        onJumpToAgent={onJumpToAgent}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: GUIDE_PRESETS[0].label }));

    await waitFor(() => expect(onJumpToAgent).toHaveBeenCalledWith('g1'));
    expect(window.electronAPI.claudeMessage).toHaveBeenCalledWith(
      'sess-guide',
      GUIDE_PRESETS[0].prompt,
    );
    expect(spawnGuide).not.toHaveBeenCalled();
    // And the shortcut row advertises the running guide.
    expect(screen.getByText(/Your guide is running/)).toBeInTheDocument();
  });

  it('a STOPPED guide (no sessionId) is not reused — a fresh spawn happens', async () => {
    const spawnGuide = vi.fn().mockResolvedValue('fresh-id');
    const stopped = agent({ id: 'g1', name: GUIDE_AGENT_NAME, sessionId: undefined });
    render(<GuidePane agents={[stopped]} spawnGuide={spawnGuide} onJumpToAgent={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: GUIDE_PRESETS[0].label }));

    await waitFor(() => expect(spawnGuide).toHaveBeenCalled());
    expect(window.electronAPI.claudeMessage).not.toHaveBeenCalled();
  });

  it('surfaces a spawn failure as an error message', async () => {
    const spawnGuide = vi.fn().mockRejectedValue(new Error('daemon offline'));
    render(<GuidePane agents={[]} spawnGuide={spawnGuide} onJumpToAgent={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: GUIDE_PRESETS[0].label }));

    expect(await screen.findByText('daemon offline')).toBeInTheDocument();
  });
});

describe('buildGuideKickoff', () => {
  it('frames the question with the guide role and keeps the question verbatim', () => {
    const kickoff = buildGuideKickoff('  how do jobs work?  ');
    expect(kickoff).toMatch(/Workspacer guide/);
    expect(kickoff).toMatch(/help/);
    expect(kickoff.endsWith('how do jobs work?')).toBe(true);
  });
});
