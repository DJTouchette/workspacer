import { afterEach, describe, expect, it, vi } from 'vitest';
import React from 'react';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { bindManagerReplacements, managerReplacementRequest } from './managerReplacement';
import { ManagerHandoffStatus } from '../components/claude/ManagerHandoffStatus';
import type { AgentWorkspace } from '../types/pane';
import type { ManagerReplacementView } from '../../../main/shared/managerReplacement';

const operation = (): ManagerReplacementView => ({
  operationId: 'operation',
  sourceSessionId: 'source',
  successorSessionId: 'successor',
  paneId: 'same-pane',
  workspaceId: 'same-workspace',
  phase: 'binding',
  createdAt: 1,
  updatedAt: 2,
  committed: true,
  bound: false,
  artifactPath: '/fixture/handoff.json',
  workerIds: ['worker'],
  taskIds: ['ordinary', 'workflow'],
  deliveries: [],
});
const agent = (overrides = {}): AgentWorkspace => ({
  id: 'same-workspace',
  name: 'Fleet Manager',
  manager: true,
  sessionId: 'source',
  cwd: '/fixture',
  activeTabId: 'same-tab',
  tabs: [
    {
      id: 'same-tab',
      title: 'Manager',
      activePaneId: 'same-pane',
      panes: [
        { id: 'same-pane', type: 'claude', title: 'Manager', attachSessionId: 'source' },
        { id: 'other-pane', type: 'browser', title: 'Docs', url: 'https://example.test' },
      ],
    },
  ],
  ...overrides,
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
describe('same-pane manager replacement binding', () => {
  it('keeps workspace/tab/pane identity and unrelated panes; no second agent card', () => {
    const source = agent();
    const unrelated = agent({ id: 'other', sessionId: 'other-session', tabs: [] });
    const prematureAdoption = agent({ id: 'accidental-card', sessionId: 'successor', tabs: [] });
    const next = bindManagerReplacements([source, unrelated, prematureAdoption], [operation()]);
    expect(next.map((a) => a.id)).toEqual(['same-workspace', 'other']);
    expect(next[0]).toMatchObject({
      id: source.id,
      sessionId: 'successor',
      activeTabId: 'same-tab',
    });
    expect(next[0].tabs[0]).toMatchObject({ id: 'same-tab', activePaneId: 'same-pane' });
    expect(next[0].tabs[0].panes[0]).toMatchObject({
      id: 'same-pane',
      attachSessionId: 'successor',
      expectHistory: false,
    });
    expect(next[0].tabs[0].panes[1]).toBe(source.tabs[0].panes[1]);
    expect(next[1]).toBe(unrelated);
    expect(bindManagerReplacements(next, [operation()])).toBe(next);
  });
  it('does not replace a pane before ownership commit or on a remote workspace', () => {
    const original = [agent()];
    expect(
      bindManagerReplacements(original, [{ ...operation(), committed: false, phase: 'failed' }]),
    ).toBe(original);
    const remote = [agent({ hub: 'peer' })];
    expect(bindManagerReplacements(remote, [operation()])).toBe(remote);
  });
  it('restores the original workspace identity after saved-layout session-id normalization', () => {
    const restored = agent({ id: 'agent-successor', sessionId: 'successor' });
    const next = bindManagerReplacements([restored], [operation()]);
    expect(next[0].id).toBe('same-workspace');
    expect(next[0].tabs[0].panes[0].attachSessionId).toBe('successor');
  });
});
describe('bounded recovery UI and API boundary', () => {
  it('old preload and remote backend responses are explicitly unavailable', async () => {
    window.electronAPI.managerReplacement = undefined;
    expect(await managerReplacementRequest({ action: 'list' })).toMatchObject({
      available: false,
      error: expect.stringContaining('local desktop'),
    });
    window.electronAPI.managerReplacement = vi
      .fn()
      .mockResolvedValue({ available: false, operations: [], error: 'Remote unavailable' });
    expect(await managerReplacementRequest({ action: 'list' })).toMatchObject({
      available: false,
      error: 'Remote unavailable',
    });
  });
  it('retains uncertain evidence and requires an explicit risk-labelled action before retry', () => {
    const request = vi.fn().mockResolvedValue({ available: true, operations: [] });
    window.electronAPI.managerReplacement = request;
    const op = {
      ...operation(),
      phase: 'recovery-required' as const,
      deliveries: [
        {
          id: 'delivery',
          kind: 'kickoff' as const,
          text: 'Start the successor using this exact handoff',
          status: 'uncertain' as const,
          error: 'Acknowledgement lost',
        },
      ],
    };
    render(<ManagerHandoffStatus operation={op} />);
    expect(screen.getByText('Manager handoff needs attention')).toBeInTheDocument();
    expect(screen.queryByText('Manager handoff complete')).not.toBeInTheDocument();
    expect(request).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText('Inspect handoff and delivery evidence'));
    fireEvent.click(screen.getByText('kickoff: uncertain'));
    expect(screen.getByText('Start the successor using this exact handoff')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Send again — may duplicate work' }));
    expect(request).toHaveBeenCalledExactlyOnceWith({
      action: 'resolve-delivery',
      operationId: 'operation',
      deliveryId: 'delivery',
      resolution: 'retry',
      acknowledgeDuplicateRisk: true,
    });
  });
});
