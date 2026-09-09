import React from 'react';
import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useProviderReadiness } from '../src/hooks/useProviderReadiness';
import FleetManagerHero from '../src/components/FleetManagerHero';
import SpawnAgentDialog from '../src/components/SpawnAgentDialog';
import SupervisorSection from '../src/components/settings/SupervisorSection';
import {
  providerReadinessDetail,
  type ProviderReadinessState,
} from '../../main/shared/providerReadiness';
const { config, refreshDetection } = vi.hoisted(() => ({
  config: { agents: { managerProvider: 'claude', checkProviderOnStartup: true } },
  refreshDetection: vi.fn(),
}));
vi.mock('../src/hooks/useConfig', () => ({ useConfig: () => ({ config, save: vi.fn() }) }));
vi.mock('../src/hooks/useProviderDetection', () => ({
  useProviderDetection: () => ({
    detection: ['claude', 'codex', 'copilot', 'opencode', 'pi'].map((provider) => ({
      provider,
      found: true,
    })),
    refresh: refreshDetection,
  }),
}));
vi.mock('../src/components/settings/HarnessModelSelect', () => ({
  default: () => null,
  useModelOptions: () => ({ options: [] }),
}));
vi.mock('../src/components/settings/FleetWorkflowsSection', () => ({ default: () => null }));
const api = window.electronAPI as any;
let listeners: Set<() => void>;
beforeEach(() => {
  vi.clearAllMocks();
  listeners = new Set();
  api.onConfigChanged = vi.fn((f: () => void) => {
    listeners.add(f);
    return () => listeners.delete(f);
  });
  api.providerReadiness = vi.fn(async () => ({ state: 'unchecked' }));
  api.agentRuntimeStatus = vi.fn(async () => ({
    claudemon: 'ready',
    hub: 'ready',
    facade: 'ready',
  }));
  api.claudeProfilesList = vi.fn(async () => []);
  api.claudeListModels = vi.fn(async () => ({ aliases: [], seen: [] }));
  config.agents.managerProvider = 'claude';
});
describe('readiness request context', () => {
  it('mount and remount read only; Check again explicitly pings', async () => {
    const first = renderHook(() => useProviderReadiness('claude'));
    await waitFor(() => expect(api.providerReadiness).toHaveBeenCalledWith('claude', false));
    first.unmount();
    const second = renderHook(() => useProviderReadiness('claude'));
    await act(async () => {
      await second.result.current.refresh();
    });
    expect(api.providerReadiness.mock.calls.filter(([, check]: any[]) => check)).toEqual([
      ['claude', true],
    ]);
  });
  it('provider switch rejects late results', async () => {
    let finish!: (r: unknown) => void;
    api.providerReadiness.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const hook = renderHook(({ provider }) => useProviderReadiness(provider), {
      initialProps: { provider: 'claude' },
    });
    hook.rerender({ provider: 'codex' });
    await act(async () => {
      finish({ state: 'responding' });
    });
    expect(hook.result.current.status.state).toBe('unchecked');
  });
  it('config change immediately invalidates old results and supersedes in-flight work', async () => {
    api.providerReadiness.mockResolvedValueOnce({ state: 'responding' });
    const hook = renderHook(() => useProviderReadiness('claude'));
    await waitFor(() => expect(hook.result.current.status.state).toBe('responding'));
    let finish!: (r: unknown) => void;
    api.providerReadiness.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    let pending!: Promise<void>;
    act(() => {
      pending = hook.result.current.refresh();
    });
    act(() => {
      for (const fn of listeners) fn();
    });
    await act(async () => {
      finish({ state: 'responding' });
      await pending;
    });
    expect(hook.result.current.status.state).toBe('unchecked');
  });
  it.each([
    { owner: 'peer-a', account: '' },
    { owner: '', account: 'profile-a' },
    { owner: '', account: 'integration-a' },
  ])('never applies local auth to %j', async ({ owner, account }) => {
    const hook = renderHook(() => useProviderReadiness('claude', owner, account));
    await act(async () => {
      await hook.result.current.refresh();
    });
    expect(hook.result.current.status.state).toBe('unsupported');
    expect(api.providerReadiness).not.toHaveBeenCalled();
  });
  it('remote switch hides known local success synchronously', async () => {
    api.providerReadiness.mockResolvedValue({ state: 'responding' });
    const hook = renderHook(({ owner }) => useProviderReadiness('claude', owner), {
      initialProps: { owner: '' },
    });
    await waitFor(() => expect(hook.result.current.status.state).toBe('responding'));
    hook.rerender({ owner: 'peer' });
    expect(hook.result.current.status.state).toBe('unsupported');
  });
  it('absent, rejected and future-version answers remain unknown', async () => {
    for (const answer of [undefined, { state: 'future' }]) {
      api.providerReadiness.mockResolvedValue(answer);
      const hook = renderHook(() => useProviderReadiness('claude'));
      await act(async () => {
        await hook.result.current.refresh();
      });
      expect(hook.result.current.status.state).toBe('unchecked');
      hook.unmount();
    }
    api.providerReadiness = undefined;
    const hook = renderHook(() => useProviderReadiness('claude'));
    await act(async () => {
      await hook.result.current.refresh();
    });
    expect(hook.result.current.status.state).toBe('unchecked');
  });
});
describe('shared advisory presentation', () => {
  it.each([
    'responding',
    'unauthenticated',
    'limited',
    'network-error',
    'timeout',
    'error',
    'unsupported',
    'unchecked',
  ] as ProviderReadinessState[])('Fleet %s never gates launch', async (state) => {
    api.providerReadiness.mockResolvedValue({ state });
    render(<FleetManagerHero />);
    await screen.findByText(providerReadinessDetail({ state }));
    fireEvent.change(screen.getByRole('textbox', { name: 'Ask the Fleet Manager' }), {
      target: { value: 'fixture task' },
    });
    expect(screen.getByRole('button', { name: 'Ask Fleet Manager' })).toBeEnabled();
    expect(screen.getByText('Fleet Manager runtime is ready.')).toBeTruthy();
    expect(screen.queryByText(/Provider sign-in may still be required/)).toBeNull();
  });
  it('Spawn displays the same success and its manual button checks auth and installation', async () => {
    api.providerReadiness.mockResolvedValue({ state: 'responding' });
    render(
      <SpawnAgentDialog
        defaultCwd="/fixture"
        defaultProvider="claude"
        defaultTransport="stream"
        defaultPrompt="fixture task"
        onSpawn={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    await screen.findByText(providerReadinessDetail({ state: 'responding' }));
    fireEvent.click(screen.getByRole('button', { name: 'Check again', exact: true }));
    await waitFor(() => expect(api.providerReadiness).toHaveBeenCalledWith('claude', true));
    expect(refreshDetection).toHaveBeenCalled();
  });
  it('toggle advertises allowance use and writes the persisted agent setting', () => {
    const save = vi.fn(async () => config as any);
    render(<SupervisorSection config={config as any} save={save} />);
    const checkbox = screen.getByRole('checkbox', { name: 'Check provider at startup' });
    expect(checkbox).toBeChecked();
    expect(screen.getByText(/may consume provider allowance/)).toBeTruthy();
    fireEvent.click(checkbox);
    expect(save).toHaveBeenCalledWith({
      agents: { ...config.agents, checkProviderOnStartup: false },
    });
  });
});
