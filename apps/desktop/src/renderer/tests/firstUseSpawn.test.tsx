import React from 'react';
import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useAgentManager } from '../src/hooks/useAgentManager';
import SpawnAgentDialog from '../src/components/SpawnAgentDialog';
import {
  __resetProviderDetectionCache,
  useProviderDetection,
} from '../src/hooks/useProviderDetection';
import { providerAvailability } from '../src/lib/providerAvailability';

const api = window.electronAPI as any;
beforeEach(() => {
  vi.clearAllMocks();
  __resetProviderDetectionCache();
  api.providerCheckAll = vi.fn().mockResolvedValue([]);
  api.spawnClaude = vi.fn().mockResolvedValue('real-session');
  api.claudeListModels = vi
    .fn()
    .mockResolvedValue({ aliases: [], seen: [], skipPermissionsDefault: false });
  api.claudeProfilesList = vi.fn().mockResolvedValue([]);
});

describe('first-use spawn outcome', () => {
  for (const failure of ['reject', undefined, null, '', ' ', {}, { sessionId: 'wrong-shape' }]) {
    it(`leaves roster and selection untouched for ${JSON.stringify(failure)}`, async () => {
      const { result } = renderHook(() => useAgentManager());
      const before = { agents: result.current.agents, active: result.current.activeAgentId };
      if (failure === 'reject') api.spawnClaude.mockRejectedValueOnce(new Error('token=secret'));
      else api.spawnClaude.mockResolvedValueOnce(failure);
      await act(async () => {
        await expect(
          result.current.spawnAgent({ cwd: '/repo', provider: 'codex', kickoffMessage: 'task' }),
        ).rejects.toThrow('Codex could not start');
      });
      expect(result.current.agents).toBe(before.agents);
      expect(result.current.activeAgentId).toBe(before.active);
      expect(api.saveConfig).not.toHaveBeenCalled();
      expect(api.claudeMessage).not.toHaveBeenCalled();
      await act(async () => {
        await result.current.spawnAgent({
          cwd: '/repo',
          provider: 'codex',
          kickoffMessage: 'task',
        });
      });
      expect(result.current.agents.filter((a) => !a.global)).toHaveLength(1);
      expect(result.current.agents.find((a) => !a.global)?.sessionId).toBe('real-session');
      expect(api.spawnClaude.mock.calls[1][0].message).toBe('task');
      expect(api.claudeMessage).not.toHaveBeenCalled();
    });
  }
  it('converges with an early adopted session into one correctly configured card', async () => {
    const { result } = renderHook(() => useAgentManager());
    api.spawnClaude.mockImplementationOnce(async () => {
      result.current.adoptAgent({ sessionId: 'real-session', cwd: '/repo' } as any);
      return 'real-session';
    });
    await act(async () => {
      await result.current.spawnAgent({ cwd: '/repo', provider: 'codex', kickoffMessage: 'task' });
    });
    const real = result.current.agents.filter((a) => !a.global);
    expect(real).toHaveLength(1);
    expect(real[0].provider).toBe('codex');
  });
  it('retains a stopped manager on failed resume and queues its retry ask in the resume spawn', async () => {
    const { result } = renderHook(() => useAgentManager());
    await act(async () => {
      await result.current.spawnFleetManager('first', '/repo');
    });
    act(() => result.current.stopAgentForSession('real-session'));
    api.spawnClaude.mockRejectedValueOnce(new Error('offline'));
    await act(async () => {
      await expect(result.current.spawnFleetManager('next', '/repo')).rejects.toThrow(
        'could not start',
      );
    });
    expect(result.current.agents.filter((a) => !a.global)).toHaveLength(1);
    await act(async () => {
      await result.current.spawnFleetManager('next', '/repo');
    });
    expect(api.spawnClaude.mock.calls.at(-1)[0]).toMatchObject({
      resumeSessionId: 'real-session',
      message: expect.stringContaining('next\n\nFor each NEW project task, call start_workflow'),
    });
    expect(api.claudeMessage).not.toHaveBeenCalled();
  });
});

it('retains the task and provider after rejection and guards concurrent submits', async () => {
  let reject!: (error: Error) => void;
  const onSpawn = vi
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise((_, fail) => {
          reject = fail;
        }),
    )
    .mockResolvedValue('id');
  render(<SpawnAgentDialog defaultCwd="/repo" requireTask onSpawn={onSpawn} onCancel={vi.fn()} />);
  const task = screen.getByLabelText('What should this agent do?');
  const launch = screen.getByRole('button', { name: 'Dispatch agent' });
  expect(launch).toBeDisabled();
  fireEvent.change(task, { target: { value: 'one task' } });
  fireEvent.click(launch);
  fireEvent.keyDown(task, { key: 'Enter', ctrlKey: true });
  expect(onSpawn).toHaveBeenCalledTimes(1);
  await act(async () => reject(new Error('secret=never render this')));
  expect(screen.getByRole('alert')).not.toHaveTextContent('secret');
  expect(task).toHaveValue('one task');
  fireEvent.click(screen.getByRole('button', { name: 'Retry dispatch' }));
  expect(onSpawn).toHaveBeenCalledTimes(2);
  expect(onSpawn.mock.calls[1][0]).toMatchObject({
    kickoffMessage: 'one task',
    permissionMode: 'default',
    skipPermissions: false,
  });
  expect(onSpawn.mock.calls[1][0]).not.toHaveProperty('initialPrompt');
});

it('creates ordinary agents without a task input or initial message', () => {
  const onSpawn = vi.fn();
  render(<SpawnAgentDialog defaultCwd="/repo" onSpawn={onSpawn} onCancel={vi.fn()} />);
  expect(screen.queryByLabelText('What should this agent do?')).not.toBeInTheDocument();
  expect(screen.queryByLabelText(/Allow an empty session/)).not.toBeInTheDocument();
  expect(screen.getByLabelText('Working directory')).toHaveFocus();
  fireEvent.click(screen.getByRole('button', { name: 'Create agent' }));
  expect(onSpawn).toHaveBeenCalledTimes(1);
  expect(onSpawn.mock.calls[0][0]).not.toHaveProperty('kickoffMessage');
  expect(onSpawn.mock.calls[0][0]).not.toHaveProperty('initialPrompt');
});

it('a failed recheck clears a stale missing verdict and a malformed row stays unknown', async () => {
  api.providerCheckAll
    .mockResolvedValueOnce([{ provider: 'claude', found: false }])
    .mockRejectedValueOnce(new Error('old host'));
  const { result } = renderHook(() => useProviderDetection());
  await waitFor(() =>
    expect(providerAvailability(result.current.detection, 'claude')).toBe('missing'),
  );
  act(() => result.current.refresh());
  await waitFor(() =>
    expect(providerAvailability(result.current.detection, 'claude')).toBe('unknown'),
  );
  expect(providerAvailability([{ provider: 'claude' } as any], 'claude')).toBe('unknown');
  expect(api.providerCheckAll).toHaveBeenLastCalledWith(true);
});

it('forces a recheck even when an earlier cached host scan is still pending', async () => {
  let answer!: (rows: any[]) => void;
  api.providerCheckAll
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    )
    .mockResolvedValueOnce([{ provider: 'claude', found: true }]);
  const { result } = renderHook(() => useProviderDetection());
  await waitFor(() => expect(api.providerCheckAll).toHaveBeenCalledTimes(1));
  act(() => {
    result.current.refresh();
    result.current.refresh();
  });
  await act(async () => answer([{ provider: 'claude', found: false }]));
  await waitFor(() =>
    expect(providerAvailability(result.current.detection, 'claude')).toBe('installed'),
  );
  expect(api.providerCheckAll).toHaveBeenCalledTimes(2);
  expect(api.providerCheckAll).toHaveBeenLastCalledWith(true);
});

it('retains the selected integration in the workspace and sends it again on Codex resume', async () => {
  api.spawnClaude.mockResolvedValue('integration-session');
  const { result } = renderHook(() => useAgentManager());
  await act(async () => {
    await result.current.spawnAgent({
      cwd: '/repo',
      provider: 'codex',
      launchIntegrationId: 'workspacer.headroom',
    });
  });
  const agent = result.current.agents.find((a) => !a.global)!;
  expect(agent.launchIntegrationId).toBe('workspacer.headroom');
  act(() => result.current.stopAgentForSession('integration-session'));
  await act(async () => {
    await result.current.respawnAgent(agent.id);
  });
  expect(api.spawnClaude.mock.calls.at(-1)[0]).toMatchObject({
    provider: 'codex',
    launchIntegrationId: 'workspacer.headroom',
    resumeSessionId: 'integration-session',
  });
});
