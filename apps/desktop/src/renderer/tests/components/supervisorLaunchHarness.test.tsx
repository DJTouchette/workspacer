/**
 * "Ask the Fleet" must launch the harness Settings says the supervisor runs on.
 *
 * The reported symptom: Settings → Supervisor was set to Codex, a supervisor was
 * launched, and the session came up on Claude. The launcher took its harness
 * from `useState(config.supervisor?.provider ?? 'claude')` — a MOUNT-TIME
 * snapshot. Config loads asynchronously (ConfigContext starts on DEFAULT_CONFIG
 * with `loaded: false`), so a pane that mounted before the load landed — every
 * pane restored from the saved layout at boot does — kept 'claude' forever and
 * never caught up, while the settings pane happily showed Codex.
 *
 * These pin the behaviour that fixes it: the picker FOLLOWS config until you
 * pick something in it, and an explicit pick then sticks.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import React from 'react';
import AskPane from '../../src/panes/AskPane';
import type { AgentProvider } from '../../src/types/pane';

/** What the (async) config currently says, mutated between renders like a real
 *  load landing after mount. */
let configProvider: AgentProvider | undefined = 'claude';

vi.mock('../../src/hooks/useConfig', () => ({
  useConfig: () => ({
    config: { agents: configProvider ? { managerProvider: configProvider } : {} },
    save: vi.fn(),
  }),
}));

// Detection is irrelevant here beyond "everything is installed", so the picker
// offers every eligible harness.
const api = window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

beforeEach(() => {
  configProvider = 'claude';
  api.providerCheckAll = vi.fn().mockResolvedValue(
    ['claude', 'codex', 'copilot', 'opencode', 'pi'].map((provider) => ({
      provider,
      found: true,
      resolvedPath: `/usr/bin/${provider}`,
      customBin: '',
    })),
  );
  api.getSupervisorHome = vi.fn().mockResolvedValue('/home/u/.workspacer');
});

describe('Ask the Fleet — the launcher runs the configured supervisor harness', () => {
  it('spawns on config agents.managerProvider', async () => {
    configProvider = 'codex';
    const spawnAskAgent = vi.fn().mockResolvedValue('agent-1');
    render(<AskPane agents={[]} spawnAskAgent={spawnAskAgent} onJumpToAgent={vi.fn()} />);
    await waitFor(() => expect(screen.getByRole('button', { name: /Codex/ })).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /Start watcher only/i }));
    await waitFor(() => expect(spawnAskAgent).toHaveBeenCalled());
    expect(spawnAskAgent.mock.calls.at(-1)![0].provider).toBe('codex');
  });

  it('catches up when config arrives AFTER the pane mounted (the boot-restored pane)', async () => {
    // Mount with the pre-load default…
    configProvider = undefined;
    const spawnAskAgent = vi.fn().mockResolvedValue('agent-1');
    const { rerender } = render(
      <AskPane agents={[]} spawnAskAgent={spawnAskAgent} onJumpToAgent={vi.fn()} />,
    );
    // …then the real config lands, exactly as ConfigContext's load does.
    configProvider = 'codex';
    rerender(<AskPane agents={[]} spawnAskAgent={spawnAskAgent} onJumpToAgent={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /Start watcher only/i }));
    await waitFor(() => expect(spawnAskAgent).toHaveBeenCalled());
    expect(spawnAskAgent.mock.calls.at(-1)![0].provider).toBe('codex');
  });

  it('an explicit pick in the launcher overrides the configured harness', async () => {
    // The per-launch override is the reason this picker exists at all; a config
    // default must not reclaim it on the next render.
    configProvider = 'codex';
    const spawnAskAgent = vi.fn().mockResolvedValue('agent-1');
    render(<AskPane agents={[]} spawnAskAgent={spawnAskAgent} onJumpToAgent={vi.fn()} />);
    await waitFor(() => expect(screen.getByRole('button', { name: /OpenCode/ })).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /OpenCode/ }));
    fireEvent.click(screen.getByRole('button', { name: /Start watcher only/i }));
    await waitFor(() => expect(spawnAskAgent).toHaveBeenCalled());
    expect(spawnAskAgent.mock.calls.at(-1)![0].provider).toBe('opencode');
  });

  it('does not offer Pi — it ships no MCP client, so it cannot watch anything', async () => {
    // One list with Settings (lib/roleProviders). This picker used to offer Pi
    // while the settings pane refused to, so the launcher could start a
    // supervisor the settings pane says is impossible.
    render(<AskPane agents={[]} spawnAskAgent={vi.fn()} onJumpToAgent={vi.fn()} />);
    await waitFor(() => expect(screen.getByRole('button', { name: /Codex/ })).toBeTruthy());
    expect(screen.queryByRole('button', { name: /^Pi$/ })).toBeNull();
  });
});
