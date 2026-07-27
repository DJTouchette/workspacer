import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import { HandoffDialog, carryPermissionMode } from '../../src/components/claude/HandoffDialog';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';
import type { AgentProvider } from '../../src/types/pane';

/**
 * The handoff dialog spawns the SUCCESSOR agent, so its launch settings must
 * start from the source session's live values — the common case is "same
 * setup, fresh context" and that has to be zero clicks. Switching the target
 * provider must not carry a model id across harnesses (a claude alias means
 * nothing to codex) while the permission INTENT does carry.
 */

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

function renderDialog(props: {
  provider?: AgentProvider;
  snapshot?: ClaudeSessionSnapshot;
  busy?: 'agent' | 'mechanical' | null;
}) {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(
    <HandoffDialog
      provider={props.provider ?? 'claude'}
      snapshot={props.snapshot ?? snapshot()}
      cwd="/repo"
      busy={props.busy ?? null}
      onCancel={onCancel}
      onConfirm={onConfirm}
    />,
  );
  return { onConfirm, onCancel };
}

/** The labeled rows are `<select>`s; grab one by its row label. */
function selectFor(label: string): HTMLSelectElement {
  const row = screen.getByText(label).parentElement as HTMLElement;
  const el = row.querySelector('select');
  if (!el) throw new Error(`no select in the "${label}" row`);
  return el as HTMLSelectElement;
}

beforeEach(() => {
  api.claudeListModels = vi.fn().mockResolvedValue({
    aliases: [
      { value: 'opus', label: 'Opus' },
      { value: 'sonnet', label: 'Sonnet' },
    ],
    seen: [],
  });
  api.providerListModels = vi
    .fn()
    .mockResolvedValue([
      { id: 'gpt-5-codex', label: 'GPT-5 Codex', default: true, effortLevels: ['low', 'high'] },
    ]);
});

describe('carryPermissionMode', () => {
  it('translates the bypass intent across harness vocabularies', () => {
    expect(carryPermissionMode('codex', 'bypassPermissions')).toBe('yolo');
    expect(carryPermissionMode('claude', 'yolo')).toBe('bypassPermissions');
  });

  it('keeps a mode the target provider offers', () => {
    expect(carryPermissionMode('claude', 'plan')).toBe('plan');
    expect(carryPermissionMode('codex', 'ask')).toBe('ask');
  });

  it("falls back to the target's safest mode for ids it can't launch with", () => {
    // 'acceptEdits' is claude-only; 'auto' is live telemetry only (no launch flag).
    expect(carryPermissionMode('codex', 'acceptEdits')).toBe('ask');
    expect(carryPermissionMode('claude', 'auto')).toBe('default');
    expect(carryPermissionMode('claude', undefined)).toBe('default');
  });
});

describe('HandoffDialog', () => {
  it("defaults every knob to the source session's live values", async () => {
    const { onConfirm } = renderDialog({
      provider: 'claude',
      snapshot: snapshot({
        settings: { model: 'sonnet', effort: 'low', permissionMode: 'default' },
        // Live telemetry outranks what was requested at spawn.
        statusLine: { effort: 'high' },
        livePermissionMode: 'plan',
      } as Partial<ClaudeSessionSnapshot>),
    });

    await waitFor(() => expect(selectFor('model').value).toBe('sonnet'));
    expect(selectFor('effort').value).toBe('high');
    expect(selectFor('permissions').value).toBe('plan');

    fireEvent.click(screen.getByText('Hand off'));
    expect(onConfirm).toHaveBeenCalledWith({
      provider: 'claude',
      model: 'sonnet',
      effort: 'high',
      permissionMode: 'plan',
      skipPermissions: false,
      brief: 'agent',
    });
  });

  it('drops the model but carries the permission intent when the provider changes', async () => {
    const { onConfirm } = renderDialog({
      provider: 'claude',
      snapshot: snapshot({
        settings: { model: 'opus', permissionMode: 'bypassPermissions' },
      } as Partial<ClaudeSessionSnapshot>),
    });
    await waitFor(() => expect(selectFor('model').value).toBe('opus'));

    fireEvent.click(screen.getByText('Codex'));
    await waitFor(() => expect(api.providerListModels).toHaveBeenCalled());

    // A claude alias is meaningless to codex — back to the provider default.
    expect(selectFor('model').value).toBe('');
    // The bypass intent survives, in codex's own vocabulary.
    expect(selectFor('permissions').value).toBe('yolo');

    fireEvent.click(screen.getByText('Hand off'));
    expect(onConfirm).toHaveBeenCalledWith({
      provider: 'codex',
      model: undefined,
      effort: undefined,
      permissionMode: 'yolo',
      skipPermissions: true,
      brief: 'agent',
    });
  });

  it("restores the source model when the target switches back to the source's provider", async () => {
    renderDialog({
      provider: 'claude',
      snapshot: snapshot({ settings: { model: 'opus' } } as Partial<ClaudeSessionSnapshot>),
    });
    await waitFor(() => expect(selectFor('model').value).toBe('opus'));

    fireEvent.click(screen.getByText('Codex'));
    await waitFor(() => expect(selectFor('model').value).toBe(''));
    fireEvent.click(screen.getByText('Claude Code'));
    await waitFor(() => expect(selectFor('model').value).toBe('opus'));
  });

  it('keeps a source model that is not in the catalog as its own row', async () => {
    renderDialog({
      provider: 'claude',
      snapshot: snapshot({
        settings: { model: 'claude-opus-4-8-20250101' },
      } as Partial<ClaudeSessionSnapshot>),
    });
    // Not an alias and not in `seen` — it must still be the selected value
    // rather than being silently reset to the provider default.
    await waitFor(() => expect(selectFor('model').value).toBe('claude-opus-4-8-20250101'));
    expect(screen.getByText('opus-4-8')).toBeTruthy();
  });

  it('picks the brief author and blocks a second submit while one is in flight', async () => {
    const { onConfirm } = renderDialog({ provider: 'claude' });
    await waitFor(() => expect(api.claudeListModels).toHaveBeenCalled());

    fireEvent.click(screen.getByText('digest'));
    fireEvent.click(screen.getByText('Hand off'));
    expect(onConfirm).toHaveBeenCalledWith(expect.objectContaining({ brief: 'mechanical' }));
  });

  it('cannot be confirmed or cancelled while the brief is being written', async () => {
    const { onConfirm, onCancel } = renderDialog({ provider: 'claude', busy: 'agent' });
    await waitFor(() => expect(api.claudeListModels).toHaveBeenCalled());

    expect(screen.getByText('Waiting for the agent to write its brief…')).toBeTruthy();
    fireEvent.click(screen.getByText('Hand off'));
    fireEvent.click(screen.getByText('Cancel'));
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onConfirm).not.toHaveBeenCalled();
    expect(onCancel).not.toHaveBeenCalled();
  });
});
