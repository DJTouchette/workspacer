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
    const row = within(screen.getByText('Supervisor runs on').closest('div') as HTMLElement);
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
    const row = within(screen.getByText('Supervisor runs on').closest('div') as HTMLElement);
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
    const row = within(screen.getByText('Supervisor runs on').closest('div') as HTMLElement);
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

/**
 * The Fleet Manager had no model setting at all — `agents.managerProvider`
 * shipped without a twin — so the manager always ran on its harness's default
 * with no way to choose. Its picker has to follow the manager harness, which is
 * a DIFFERENT harness from the supervisor's: both live in this one section, and
 * a picker reading the wrong one is exactly the bug the supervisor row already
 * had.
 */
describe('SupervisorSection — the Fleet Manager model picker', () => {
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

  it('follows agents.managerProvider, not supervisor.provider', async () => {
    // Supervisor on claude, manager on codex: the manager row must show CODEX
    // models even though the supervisor row above it is showing Claude ones.
    render(
      <SupervisorSection
        config={
          {
            supervisor: { provider: 'claude' },
            agents: { managerProvider: 'codex' },
          } as Config
        }
        save={vi.fn().mockResolvedValue({})}
      />,
    );
    const row = within(screen.getByText('Manager model').closest('div') as HTMLElement);
    fireEvent.click(row.getByRole('button'));
    await waitFor(() => expect(row.getByText('GPT-5 Codex')).toBeInTheDocument());
    expect(row.queryByText('Fable')).not.toBeInTheDocument();
  });

  it('writes the choice under the manager harness, leaving the other harness alone', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(
      <SupervisorSection
        config={
          {
            agents: { managerProvider: 'codex', managerModels: { claude: 'fable' } },
          } as Config
        }
        save={save}
      />,
    );
    const row = within(screen.getByText('Manager model').closest('div') as HTMLElement);
    fireEvent.click(row.getByRole('button'));
    await waitFor(() => expect(row.getByText('GPT-5 Codex')).toBeInTheDocument());
    fireEvent.click(row.getByText('GPT-5 Codex'));
    await waitFor(() => expect(save).toHaveBeenCalled());
    const patched = save.mock.calls.at(-1)![0] as {
      agents: { managerModels: Record<string, string> };
    };
    expect(patched.agents.managerModels).toEqual({ claude: 'fable', codex: 'gpt-5-codex' });
  });

  it('reads back the entry for the current harness only', async () => {
    render(
      <SupervisorSection
        config={
          {
            agents: {
              managerProvider: 'claude',
              managerModels: { claude: 'fable', codex: 'gpt-5-codex' },
            },
          } as Config
        }
        save={vi.fn().mockResolvedValue({})}
      />,
    );
    const row = within(screen.getByText('Manager model').closest('div') as HTMLElement);
    await waitFor(() => expect(row.getByRole('button').textContent).toMatch(/Fable|fable/));
  });
});

/**
 * The summarizer picker used to be pinned to CLAUDE's catalog whatever harness
 * the supervisor ran on, and correctly so at the time: the digest worker was
 * spawned through the facade with no provider, which spawns Claude. Now that the
 * worker follows its supervisor's harness, the picker has to as well — and the
 * shipped `'sonnet'` must not be displayed as codex's configured summarizer.
 */
describe('SupervisorSection — the summarizer picker follows the supervisor harness', () => {
  beforeEach(() => {
    const api = window.electronAPI as unknown as Record<string, unknown>;
    api.claudeListModels = vi.fn().mockResolvedValue({
      aliases: [{ value: 'sonnet', label: 'Sonnet' }],
      seen: [],
      defaultModel: '',
    });
    api.providerListModels = vi
      .fn()
      .mockResolvedValue([{ id: 'gpt-5-codex', label: 'GPT-5 Codex' }]);
  });

  it('offers codex models when the supervisor is codex', async () => {
    render(
      <SupervisorSection
        config={{ supervisor: { provider: 'codex', summarizerModel: 'sonnet' } } as Config}
        save={vi.fn().mockResolvedValue({})}
      />,
    );
    const row = within(screen.getByText('Summarizer model').closest('div') as HTMLElement);
    fireEvent.click(row.getByRole('button'));
    await waitFor(() => expect(row.getByText('GPT-5 Codex')).toBeInTheDocument());
    expect(row.queryByText('Sonnet')).not.toBeInTheDocument();
  });

  it("does not present the claude-shaped 'sonnet' as codex's configured summarizer", async () => {
    const { container } = render(
      <SupervisorSection
        config={{ supervisor: { provider: 'codex', summarizerModel: 'sonnet' } } as Config}
        save={vi.fn().mockResolvedValue({})}
      />,
    );
    const row = within(screen.getByText('Summarizer model').closest('div') as HTMLElement);
    // Blank = "codex default", which is what main resolves it to.
    expect(row.getByRole('button').textContent).toMatch(/codex default/i);
    expect(container.textContent).not.toMatch(/sonnet.*belongs to a different harness/);
  });

  it('writes per-harness, and keeps the legacy field in step only where servable', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(
      <SupervisorSection
        config={{ supervisor: { provider: 'codex', summarizerModel: 'sonnet' } } as Config}
        save={save}
      />,
    );
    const row = within(screen.getByText('Summarizer model').closest('div') as HTMLElement);
    fireEvent.click(row.getByRole('button'));
    await waitFor(() => expect(row.getByText('GPT-5 Codex')).toBeInTheDocument());
    fireEvent.click(row.getByText('GPT-5 Codex'));
    await waitFor(() => expect(save).toHaveBeenCalled());
    const patched = save.mock.calls.at(-1)![0] as {
      supervisor: { summarizerModels: Record<string, string>; summarizerModel?: string };
    };
    expect(patched.supervisor.summarizerModels).toEqual({ codex: 'gpt-5-codex' });
    // The legacy single field is claude-shaped by design; a codex id may take it
    // over only because nothing else claims it — what must never happen is the
    // reverse, a claude id being rewritten to mean codex.
    expect(patched.supervisor.summarizerModel).not.toBe('sonnet');
  });
});

/**
 * The pane's two invariants, and the confusion that motivated them.
 *
 * Two DIFFERENT agents are configured here — the Fleet Manager (started from
 * the dashboard) and the supervisor (started from "Ask the Fleet") — and the
 * pane used to be titled "Supervisor" with both harness rows labelled "… agent".
 * Setting "Supervisor agent" to Codex and then launching the manager therefore
 * produced a Claude agent, correctly, and read as the setting being ignored.
 *
 * Invariant 1: no control exists that a spawn path ignores.
 * Invariant 2: no setting a spawn path honours is missing from the pane. Effort
 * is the one that was missing outright — both roles pass a `--effort`/
 * `model_reasoning_effort` through to the CLI and neither had a way to set it.
 */
describe('SupervisorSection — the two roles are told apart', () => {
  it('names both roles and says what starts each', () => {
    const { container } = renderSection();
    // (Each name appears twice: the role heading, and the launcher it names in
    // the sentence under it.)
    expect(screen.getAllByText('Fleet Manager').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Supervisor').length).toBeGreaterThan(0);
    // Each harness row says WHICH role it is for, rather than both reading
    // "… agent" as they used to.
    expect(screen.getByText('Manager runs on')).toBeInTheDocument();
    expect(screen.getByText('Supervisor runs on')).toBeInTheDocument();
    expect(container.textContent).toMatch(/Overview dashboard/);
    expect(container.textContent).toMatch(/Ask the Fleet/);
  });
});

describe('SupervisorSection — reasoning effort, per role and per harness', () => {
  it('writes the supervisor level under the supervisor harness only', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(
      <SupervisorSection
        config={
          {
            supervisor: { provider: 'codex', efforts: { claude: 'high' } },
          } as Config
        }
        save={save}
      />,
    );
    const row = within(
      screen.getByText('Supervisor thinking effort').closest('div') as HTMLElement,
    );
    fireEvent.click(row.getByRole('button'));
    await waitFor(() => expect(row.getByText('Extra high')).toBeInTheDocument());
    fireEvent.click(row.getByText('Extra high'));
    await waitFor(() => expect(save).toHaveBeenCalled());
    const patched = save.mock.calls.at(-1)![0] as {
      supervisor: { efforts: Record<string, string> };
    };
    // The other harness's choice survives — same per-harness memory as models.
    expect(patched.supervisor.efforts).toEqual({ claude: 'high', codex: 'xhigh' });
  });

  it('writes the manager level under the manager harness, independently', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(
      <SupervisorSection
        config={
          {
            supervisor: { provider: 'claude' },
            agents: { managerProvider: 'codex' },
          } as Config
        }
        save={save}
      />,
    );
    const row = within(screen.getByText('Manager thinking effort').closest('div') as HTMLElement);
    fireEvent.click(row.getByRole('button'));
    await waitFor(() => expect(row.getByText('Extra high')).toBeInTheDocument());
    fireEvent.click(row.getByText('Extra high'));
    await waitFor(() => expect(save).toHaveBeenCalled());
    const patched = save.mock.calls.at(-1)![0] as {
      agents: { managerEfforts: Record<string, string> };
    };
    expect(patched.agents.managerEfforts).toEqual({ codex: 'xhigh' });
  });

  it('reads back the level for the current harness only', () => {
    render(
      <SupervisorSection
        config={
          {
            supervisor: { provider: 'codex', efforts: { claude: 'high', codex: 'low' } },
          } as Config
        }
        save={vi.fn().mockResolvedValue({})}
      />,
    );
    const row = within(
      screen.getByText('Supervisor thinking effort').closest('div') as HTMLElement,
    );
    expect(row.getByRole('button').textContent).toMatch(/Low/);
  });

  it('shows NO effort row for a harness with no such knob', () => {
    // OpenCode has `effort: null` in providerCaps. A row that could never be
    // honoured is worse than no row: it is a control the spawn path ignores.
    render(
      <SupervisorSection
        config={{ supervisor: { provider: 'opencode' } } as Config}
        save={vi.fn().mockResolvedValue({})}
      />,
    );
    expect(screen.queryByText('Supervisor thinking effort')).toBeNull();
    // …while the manager's own row (claude) is unaffected.
    expect(screen.getByText('Manager thinking effort')).toBeInTheDocument();
  });
});
