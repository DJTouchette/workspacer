/**
 * Pi core ships no MCP client at all (pi.rs), so `managedSpawn.ts` refuses to
 * mint it a facade token (`provider !== 'pi'`) and `agentSkillsRoot` returns
 * null for it, meaning a Pi supervisor gets role instructions and nothing
 * else: no fleet-observation tools, no /supervise skill. The settings copy
 * used to claim otherwise ("Codex, OpenCode, and Pi supervisors are wired to
 * the workspacer MCP facade … via their own MCP config") and the picker
 * offered Pi as a supervisor harness anyway. MANAGER_PROVIDERS already
 * excludes Pi from the manager picker for the identical reason — this file
 * pins the supervisor picker to the same rule.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within, waitFor, fireEvent } from '@testing-library/react';
import React from 'react';
import SupervisorSection from '../../src/components/settings/SupervisorSection';
import type { Config } from '../../src/hooks/useConfig';

function renderSection(config: Partial<Config> = {}) {
  return render(<SupervisorSection config={config as Config} save={vi.fn()} />);
}

describe('SupervisorSection — Pi has no facade access, so it must not be offered or claimed', () => {
  it('does not offer Pi as a supervisor harness', () => {
    renderSection();
    const row = within(screen.getByText('Supervisor agent').closest('div') as HTMLElement);
    expect(row.queryByRole('button', { name: 'Pi' })).not.toBeInTheDocument();
    expect(row.getByRole('button', { name: 'Claude' })).toBeInTheDocument();
    expect(row.getByRole('button', { name: 'Codex' })).toBeInTheDocument();
    expect(row.getByRole('button', { name: 'OpenCode' })).toBeInTheDocument();
  });

  it('does not claim Pi supervisors are wired to the workspacer MCP facade', () => {
    const { container } = renderSection();
    // The false claim this pins: Pi cannot be "wired to the workspacer MCP
    // facade via their own MCP config" the way Codex and OpenCode genuinely
    // are (codex.rs / opencode.rs write real MCP config; pi.rs warns facade
    // tools are unavailable to it).
    expect(container.textContent).not.toMatch(/\bPi\b/);
  });
});

/**
 * The reported bug: "in the settings when I click codex I still get Claude
 * models." `useModelOptions` called `claudeListModels()` unconditionally, so
 * the Supervisor model dropdown offered Claude aliases whatever harness was
 * selected — and saving one wrote an id the codex CLI rejects at spawn.
 */
describe('SupervisorSection — the model picker follows the selected harness', () => {
  beforeEach(() => {
    const api = window.electronAPI as unknown as Record<string, unknown>;
    api.claudeListModels = vi.fn().mockResolvedValue({
      aliases: [{ value: 'fable', label: 'Fable' }],
      seen: [],
      defaultModel: '',
    });
    api.providerListModels = vi
      .fn()
      .mockResolvedValue([{ id: 'gpt-5-codex', label: 'GPT-5 Codex' }]);
  });

  it('lists CODEX models — not Claude aliases — when the harness is codex', async () => {
    render(
      <SupervisorSection
        config={{ supervisor: { provider: 'codex' } } as Config}
        save={vi.fn().mockResolvedValue({})}
      />,
    );
    await waitFor(() => expect(window.electronAPI.providerListModels).toHaveBeenCalled());
    expect(window.electronAPI.providerListModels).toHaveBeenCalledWith('codex', undefined);
    // Open the combobox: SearchableSelect renders its options only while open.
    const row = within(screen.getByText('Supervisor model').closest('div') as HTMLElement);
    fireEvent.click(row.getByRole('button'));
    await waitFor(() => expect(row.getByText('GPT-5 Codex')).toBeInTheDocument());
    expect(row.queryByText('Fable')).not.toBeInTheDocument();
  });

  it('flags a saved model the selected harness does not offer', async () => {
    const { container } = render(
      <SupervisorSection
        config={{ supervisor: { provider: 'codex', model: 'fable' } } as Config}
        save={vi.fn().mockResolvedValue({})}
      />,
    );
    await waitFor(() => expect(container.textContent).toMatch(/is not in codex’s model list/));
  });

  it('switching harness swaps the model instead of leaving a foreign id selected', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(
      <SupervisorSection
        config={{ supervisor: { provider: 'claude', model: 'fable' } } as Config}
        save={save}
      />,
    );
    const row = within(screen.getByText('Supervisor agent').closest('div') as HTMLElement);
    fireEvent.click(row.getByRole('button', { name: 'Codex' }));
    // The outgoing choice is filed under claude (so flipping back restores it)
    // and codex starts on its own default rather than inheriting `fable`.
    expect(save).toHaveBeenCalledWith({
      supervisor: { provider: 'codex', model: '', models: { claude: 'fable' } },
    });
  });

  it('remembers each harness’s model across a round trip', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(
      <SupervisorSection
        config={
          {
            supervisor: {
              provider: 'codex',
              model: 'gpt-5-codex',
              models: { claude: 'fable', codex: 'gpt-5-codex' },
            },
          } as Config
        }
        save={save}
      />,
    );
    const row = within(screen.getByText('Supervisor agent').closest('div') as HTMLElement);
    fireEvent.click(row.getByRole('button', { name: 'Claude' }));
    expect(save).toHaveBeenCalledWith({
      supervisor: {
        provider: 'claude',
        model: 'fable',
        models: { claude: 'fable', codex: 'gpt-5-codex' },
      },
    });
  });
});
