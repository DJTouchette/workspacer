import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import { ComposerControls } from '../../src/components/claude/ComposerControls';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';
import type { AgentProvider } from '../../src/types/pane';

/**
 * ComposerControls renders the model / effort / permission pills and owns the
 * live-switch vs restart decision per provider (see lib/providerCaps.ts). The
 * pills must reflect the session's reported state, and the live-switch paths
 * must call the exact daemon endpoints — claudeMessage("/model …") for claude,
 * claudeSetPermissionMode for the permission mode.
 */

// These methods aren't in tests/setup.ts's base mock; add them so the pills can
// load model lists and drive live switches.
const api = window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

function snapshot(overrides: Partial<ClaudeSessionSnapshot> = {}): ClaudeSessionSnapshot {
  return {
    sessionId: 'sess-1',
    cwd: '/repo',
    status: 'active',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    pendingApproval: null,
    subagents: [],
    ambientState: 'idle',
    lastActivity: Date.now(),
    totalToolCalls: 0,
    ...overrides,
  } as ClaudeSessionSnapshot;
}

function renderControls(
  props: {
    provider?: AgentProvider;
    sessionId?: string | null;
    snapshot?: ClaudeSessionSnapshot | null;
  } = {},
) {
  const onRestartWith = vi.fn();
  render(
    <ComposerControls
      provider={props.provider ?? 'claude'}
      sessionId={props.sessionId === undefined ? 'sess-1' : props.sessionId}
      snapshot={props.snapshot ?? snapshot()}
      cwd="/repo"
      onRestartWith={onRestartWith}
    />,
  );
  return { onRestartWith };
}

beforeEach(() => {
  api.claudeListModels = vi.fn().mockResolvedValue({
    defaultModel: 'sonnet',
    skipPermissionsDefault: false,
    defaultPermissionMode: '',
    aliases: [
      { value: 'opus', label: 'Opus' },
      { value: 'sonnet', label: 'Sonnet' },
    ],
    seen: [],
  });
  api.providerListModels = vi.fn().mockResolvedValue([
    { id: 'gpt-5-codex', label: 'GPT-5 Codex', default: true },
    { id: 'o3', label: 'o3', default: false },
  ]);
  api.claudeSetModel = vi.fn().mockResolvedValue({ ok: true });
  api.claudeSetPermissionMode = vi.fn().mockResolvedValue({ ok: true });
  api.claudeMessage = vi.fn().mockResolvedValue({ ok: true });
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('ComposerControls — pill labels reflect session state', () => {
  it('shows the live model from statusLine, and the spawn permission mode', () => {
    renderControls({
      snapshot: snapshot({
        statusLine: { modelDisplay: 'Opus 4.8' } as any,
        settings: { permissionMode: 'plan' },
      }),
    });
    expect(screen.getByText('Opus 4.8')).toBeInTheDocument();
    expect(screen.getByText('Plan mode')).toBeInTheDocument();
  });

  it('falls back to the spawn-requested model when no live telemetry has arrived', () => {
    renderControls({ snapshot: snapshot({ settings: { model: 'my-custom-model' } }) });
    expect(screen.getByText('my-custom-model')).toBeInTheDocument();
  });

  it('live permission mode (hook telemetry) wins over the frozen spawn setting', () => {
    renderControls({
      snapshot: snapshot({
        settings: { permissionMode: 'default' },
        livePermissionMode: 'acceptEdits',
      }),
    });
    expect(screen.getByText('Accept edits')).toBeInTheDocument();
    expect(screen.queryByText('Ask to approve')).not.toBeInTheDocument();
  });

  it('renders an effort pill for codex and none for claude', () => {
    const { unmount } = render(
      <ComposerControls
        provider="codex"
        sessionId="s"
        snapshot={snapshot({ settings: { effort: 'high' } })}
        cwd="/r"
        onRestartWith={vi.fn()}
      />,
    );
    expect(screen.getByText('High')).toBeInTheDocument();
    unmount();
    render(
      <ComposerControls
        provider="claude"
        sessionId="s"
        snapshot={snapshot()}
        cwd="/r"
        onRestartWith={vi.fn()}
      />,
    );
    // Claude has no effort knob, so no effort levels are shown.
    expect(screen.queryByText('High')).not.toBeInTheDocument();
  });

  it('disables the pills when there is no session yet', () => {
    renderControls({ sessionId: null });
    // Every pill is a disabled button in the no-session state.
    const buttons = screen.getAllByRole('button');
    expect(buttons.length).toBeGreaterThan(0);
    buttons.forEach((b) => expect(b).toBeDisabled());
  });
});

describe('ComposerControls — claude live switches', () => {
  it('switching a claude model sends "/model <id>" through the message path', async () => {
    renderControls({ provider: 'claude', snapshot: snapshot({ settings: { model: 'sonnet' } }) });
    // Open the model menu (the pill shows the current model label).
    fireEvent.click(screen.getByText('sonnet'));
    const opus = await screen.findByText('Opus');
    fireEvent.click(opus);
    expect(api.claudeMessage).toHaveBeenCalledWith('sess-1', '/model opus');
    // claude never routes a model change through the managed endpoint.
    expect(api.claudeSetModel).not.toHaveBeenCalled();
  });

  it('switching a claude permission mode calls claudeSetPermissionMode with the mode id', async () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { permissionMode: 'default' } }),
    });
    fireEvent.click(screen.getByText('Ask to approve'));
    const plan = await screen.findByText('Plan mode');
    fireEvent.click(plan);
    expect(api.claudeSetPermissionMode).toHaveBeenCalledWith('sess-1', 'plan');
  });
});

describe('ComposerControls — managed provider (codex)', () => {
  it('switching a codex model goes through the managed setModel endpoint', async () => {
    renderControls({
      provider: 'codex',
      snapshot: snapshot({ settings: { model: 'gpt-5-codex' } }),
    });
    fireEvent.click(screen.getByText('gpt-5-codex'));
    const o3 = await screen.findByText('o3');
    fireEvent.click(o3);
    expect(api.claudeSetModel).toHaveBeenCalledWith('sess-1', 'o3');
    expect(api.claudeMessage).not.toHaveBeenCalled();
  });

  it('picking a restart-only effort level opens a confirm and calls onRestartWith', async () => {
    const { onRestartWith } = renderControls({
      provider: 'codex',
      snapshot: snapshot({ settings: { effort: 'low' } }),
    });
    // The effort pill shows the current level.
    fireEvent.click(screen.getByText('Low'));
    fireEvent.click(await screen.findByText('High'));
    // Restart-required selection surfaces a confirm step first.
    const confirm = await screen.findByText(/Restart with High effort/);
    fireEvent.click(confirm);
    expect(onRestartWith).toHaveBeenCalledWith({ effort: 'high' });
  });

  it('a failed live model switch falls back to the restart confirm carrying the daemon reason', async () => {
    api.claudeSetModel = vi
      .fn()
      .mockResolvedValue({ ok: false, error: 'rollout fallback can’t switch live' });
    const { onRestartWith } = renderControls({
      provider: 'codex',
      snapshot: snapshot({ settings: { model: 'gpt-5-codex' } }),
    });
    fireEvent.click(screen.getByText('gpt-5-codex'));
    fireEvent.click(await screen.findByText('o3'));
    // The daemon's reason shows in the fallback confirm.
    expect(await screen.findByText(/rollout fallback/)).toBeInTheDocument();
    fireEvent.click(await screen.findByText(/Restart with o3/));
    await waitFor(() => expect(onRestartWith).toHaveBeenCalledWith({ model: 'o3' }));
  });
});
