/**
 * Settings → Fleet Manager. The pane configures ONE role now: the manager.
 *
 * Pi core ships no MCP client at all (pi.rs), so `managedSpawn.ts` refuses to
 * mint it a facade token (`provider !== 'pi'`) and `agentSkillsRoot` returns
 * null for it — MANAGER_PROVIDERS excludes it (and OpenCode) for that reason,
 * and the copy must not claim otherwise. The rest of this file pins the two
 * standing invariants: no control exists that a spawn path ignores, and no
 * setting a spawn path honours is missing from the pane.
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
  it('does not offer Pi as a manager harness', () => {
    renderSection();
    const row = within(screen.getByText('Manager runs on').closest('div') as HTMLElement);
    expect(row.queryByRole('button', { name: 'Pi' })).not.toBeInTheDocument();
    expect(row.getByRole('button', { name: 'Claude' })).toBeInTheDocument();
    expect(row.getByRole('button', { name: 'Codex' })).toBeInTheDocument();
  });

  // Copilot is the third manager harness: it has a real MCP client (servers
  // ride in on --additional-mcp-config) and a personal-skills directory
  // (~/.copilot/skills), which is the whole bar MANAGER_PROVIDERS sets.
  it('offers GitHub Copilot as a manager harness', () => {
    renderSection();
    const row = within(screen.getByText('Manager runs on').closest('div') as HTMLElement);
    expect(row.getByRole('button', { name: 'GitHub Copilot' })).toBeInTheDocument();
  });

  // Its capability surface is the only one not decidable before the session
  // starts (an org policy can disable third-party MCP servers), so the pane
  // says so — but only when it is the harness actually chosen.
  it('warns about the org MCP policy only when copilot is selected', () => {
    const policy = /org policy can disable third-party MCP servers/;
    expect(renderSection().container.textContent).not.toMatch(policy);
    expect(
      renderSection({ agents: { managerProvider: 'copilot' } } as Partial<Config>).container
        .textContent,
    ).toMatch(policy);
  });

  it('does not claim Pi agents are wired to the workspacer MCP facade', () => {
    const { container } = renderSection();
    // The false claim this pins: Pi cannot be "wired to the workspacer MCP
    // facade via its own MCP config" the way Codex genuinely is (codex.rs
    // writes real MCP config; pi.rs warns facade tools are unavailable to it).
    expect(container.textContent).not.toMatch(/\bPi\b/);
  });
});

/**
 * The Fleet Manager had no model setting at all — `agents.managerProvider`
 * shipped without a twin — so the manager always ran on its harness's default
 * with no way to choose. Its picker has to follow the manager harness — a
 * picker reading some other setting is exactly the bug the retired supervisor
 * row shipped with.
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

  it('follows agents.managerProvider', async () => {
    // Supervisor on claude, manager on codex: the manager row must show CODEX
    // models: the manager harness is what this row follows.
    render(
      <SupervisorSection
        config={
          {
            agents: { managerProvider: 'codex' },
          } as Config
        }
        save={vi.fn().mockResolvedValue({})}
      />,
    );
    const row = within(screen.getByText('Manager model').closest('div') as HTMLElement);
    fireEvent.click(row.getByRole('button', { name: /codex default/i }));
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
    fireEvent.click(row.getByRole('button', { name: /codex default/i }));
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

describe('SupervisorSection — the Fleet Manager context preference', () => {
  beforeEach(() => {
    const api = window.electronAPI as unknown as Record<string, unknown>;
    api.claudeListModels = vi.fn().mockResolvedValue({
      aliases: [
        { model: 'opus', label: 'Opus · 200K', contextWindow: 200_000 },
        { model: 'opus', label: 'Opus · 1M', contextWindow: 1_000_000 },
      ],
      seen: [],
      defaultModel: '',
    });
    api.providerListModels = vi
      .fn()
      .mockResolvedValue([{ id: 'gpt-5-codex', label: 'GPT-5 Codex' }]);
  });

  it('shows the shared fresh-Codex 1M request and persists provider-default null', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(
      <SupervisorSection
        config={
          {
            agents: {
              managerProvider: 'codex',
              managerContextWindows: { claude: 200_000 },
            },
          } as Config
        }
        save={save}
      />,
    );
    expect(screen.getByLabelText('Context settings')).toHaveTextContent('1.0M requested');
    fireEvent.click(screen.getByRole('button', { name: 'Provider default' }));
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(save.mock.calls.at(-1)![0]).toMatchObject({
      agents: { managerContextWindows: { claude: 200_000, codex: null } },
    });
  });

  it('persists a validated custom Codex token request', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(
      <SupervisorSection config={{ agents: { managerProvider: 'codex' } } as Config} save={save} />,
    );
    fireEvent.change(screen.getByLabelText('Custom context tokens'), {
      target: { value: '400000' },
    });
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(save.mock.calls.at(-1)![0]).toMatchObject({
      agents: { managerContextWindows: { codex: 400_000 } },
    });
  });

  it('offers only catalog-validated Claude context siblings', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(
      <SupervisorSection
        config={
          {
            agents: {
              managerProvider: 'claude',
              managerModels: { claude: 'opus' },
              managerContextWindows: { claude: 200_000 },
            },
          } as Config
        }
        save={save}
      />,
    );
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Opus · 1M' })).toBeInTheDocument(),
    );
    expect(screen.queryByLabelText('Custom context tokens')).not.toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Context settings'));
    expect(screen.getByRole('button', { name: 'Opus · 1M' })).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Opus · 1M' }));
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(save.mock.calls.at(-1)![0]).toMatchObject({
      agents: { managerContextWindows: { claude: 1_000_000 } },
    });
  });

  it('persists the Claude catalog model and its base context as one pair', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(
      <SupervisorSection
        config={{ agents: { managerProvider: 'claude' } } as Config}
        save={save}
      />,
    );
    const row = within(screen.getByText('Manager model').closest('div') as HTMLElement);
    fireEvent.click(row.getByRole('button', { name: /claude default/i }));
    await waitFor(() => expect(row.getByText('Opus · 200K')).toBeInTheDocument());
    fireEvent.click(row.getByText('Opus · 200K'));
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(save.mock.calls.at(-1)![0]).toMatchObject({
      agents: {
        managerModels: { claude: 'opus' },
        managerContextWindows: { claude: 200_000 },
      },
    });
  });

  it('keeps Copilot context provider-managed and read-only', () => {
    renderSection({ agents: { managerProvider: 'copilot' } } as Partial<Config>);
    expect(screen.getByLabelText('Context settings')).toHaveTextContent('provider-managed');
    expect(screen.getByText(/exposes no validated request control/)).toBeInTheDocument();
    expect(screen.queryByLabelText('Custom context tokens')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Request 1M' })).not.toBeInTheDocument();
  });
});

/**
 * The pane's two invariants, and the confusion that motivated them.
 *
 * This pane used to configure two DIFFERENT agents — the Fleet Manager (started
 * from the dashboard) and the older supervisor (started from "Ask the Fleet") —
 * with both harness rows labelled "… agent". Setting "Supervisor agent" to
 * Codex and then launching the manager therefore produced a Claude agent,
 * correctly, and read as the setting being ignored. The supervisor role is gone
 * and this pane is manager-only; that is what these pin.
 *
 * Invariant 1: no control exists that a spawn path ignores.
 * Invariant 2: no setting a spawn path honours is missing from the pane. Effort
 * is the one that was missing outright — the manager passes an `--effort` /
 * `model_reasoning_effort` through to the CLI and had no way to set it.
 */
describe('SupervisorSection — manager-only, and it says what starts the manager', () => {
  it('names the manager and where it starts, and offers no supervisor controls', () => {
    const { container } = renderSection();
    expect(screen.getAllByText('Fleet Manager').length).toBeGreaterThan(0);
    expect(screen.getByText('Manager runs on')).toBeInTheDocument();
    expect(container.textContent).toMatch(/Overview dashboard/);
    // The removed half: no second harness row, no poll interval, no summarizer.
    expect(screen.queryByText('Supervisor runs on')).toBeNull();
    expect(screen.queryByText('Supervisor model')).toBeNull();
    expect(screen.queryByText('Summarizer model')).toBeNull();
    expect(screen.queryByText('Check the fleet every')).toBeNull();
    // "Supervisor" is dead PRODUCT vocabulary here: the pane is the Fleet
    // Manager's. The one surviving occurrence is a config KEY the manager hint
    // has to name: routing.yaml's `roles.supervisor` row, which exists and is
    // not consulted for this setting. Strip that token, then the old word must
    // be gone.
    expect(container.textContent!.replace(/roles\.supervisor/g, '')).not.toMatch(/supervisor/i);
  });

  it('says the routing matrix does not pick the manager’s own model', () => {
    // Two mechanisms that both look like they choose the manager's model, with
    // neither naming the other, is a bug this project has already had once.
    // routing.yaml's comment names Settings; this is Settings naming it back.
    const { container } = renderSection();
    expect(container.textContent).toMatch(/roles\.supervisor/);
    expect(container.textContent).toMatch(/not consulted for this one/);
    expect(container.textContent).toMatch(/only place the manager’s own model is chosen/);
  });
});

describe('SupervisorSection — reasoning effort, per harness', () => {
  it('writes the manager level under the manager harness, independently', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(
      <SupervisorSection
        config={
          {
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
});
