/**
 * The auto-title model picker, which the user named explicitly:
 * `agents.autoTitle.model` shipped `'haiku'` — a Claude id — and the dropdown
 * offered Claude aliases only, so a codex-primary user had no way to name a
 * codex title model at all.
 *
 * The shape here is deliberately NOT the supervisor's. The supervisor runs on
 * ONE configured harness, so its picker switches which harness it configures.
 * Auto-titling runs on the harness of whichever agent is being titled, so this
 * row selects which harness you are EDITING and all of them stay live at once —
 * a mixed fleet needs a claude title model and a codex one simultaneously.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within, waitFor, fireEvent } from '@testing-library/react';
import React from 'react';
import SessionSection from '../../src/components/settings/SessionSection';
import type { Config } from '../../src/hooks/useConfig';

beforeEach(() => {
  const api = window.electronAPI as unknown as Record<string, unknown>;
  api.claudeListModels = vi.fn().mockResolvedValue({
    aliases: [{ value: 'haiku', label: 'Haiku' }],
    seen: [],
    defaultModel: '',
  });
  api.providerListModels = vi.fn().mockResolvedValue([{ id: 'gpt-5-nano', label: 'GPT-5 Nano' }]);
});

function renderSection(config: Partial<Config> = {}, save = vi.fn().mockResolvedValue({})) {
  return { save, ...render(<SessionSection config={config as Config} save={save} />) };
}

/** The model row for whichever harness is currently selected. */
function modelRow() {
  const label = screen.getByText(/title model$/i);
  return within(label.closest('div') as HTMLElement);
}

describe('SessionSection — the auto-title model picker follows the harness', () => {
  it('offers every harness that can answer a one-shot title call', () => {
    renderSection();
    const row = within(screen.getByText('Title model for').closest('div') as HTMLElement);
    // Copilot included: it has a directCompletion adapter like the rest, even
    // though the only id it serves is 'auto'.
    for (const name of ['Claude', 'Codex', 'Copilot', 'OpenCode', 'Pi']) {
      expect(row.getByRole('button', { name })).toBeInTheDocument();
    }
  });

  it('names the model row after a copilot-primary user’s own harness', async () => {
    // TITLE_PROVIDERS is also the label source for the row beneath it, and
    // titleHarness starts at agents.defaultProvider — so a missing entry read
    // as literally "undefined title model" with no harness button selected.
    renderSection({ agents: { defaultProvider: 'copilot' } });
    const row = within(screen.getByText('Title model for').closest('div') as HTMLElement);
    expect(row.getByRole('button', { name: 'Copilot' })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText('Copilot title model')).toBeInTheDocument());
    expect(screen.queryByText(/undefined title model/i)).toBeNull();
  });

  it('lists CODEX models once the codex row is selected', async () => {
    renderSection();
    const harnesses = within(screen.getByText('Title model for').closest('div') as HTMLElement);
    fireEvent.click(harnesses.getByRole('button', { name: 'Codex' }));
    await waitFor(() => expect(window.electronAPI.providerListModels).toHaveBeenCalled());
    const row = modelRow();
    fireEvent.click(row.getByRole('button'));
    await waitFor(() => expect(row.getByText('GPT-5 Nano')).toBeInTheDocument());
    expect(row.queryByText('Haiku')).not.toBeInTheDocument();
  });

  it("does not present the shipped claude 'haiku' as codex's configured title model", async () => {
    renderSection({ agents: { autoTitle: { model: 'haiku' } } });
    const harnesses = within(screen.getByText('Title model for').closest('div') as HTMLElement);
    fireEvent.click(harnesses.getByRole('button', { name: 'Codex' }));
    // Blank = "codex default" — exactly what main resolves it to, because
    // 'haiku' is not something codex can serve.
    await waitFor(() =>
      expect(modelRow().getByRole('button').textContent).toMatch(/codex default/i),
    );
  });

  it('writes into autoTitle.models under the selected harness, leaving the others alone', async () => {
    const { save } = renderSection({
      agents: { autoTitle: { model: 'haiku', models: { claude: 'haiku' } } },
    });
    const harnesses = within(screen.getByText('Title model for').closest('div') as HTMLElement);
    fireEvent.click(harnesses.getByRole('button', { name: 'Codex' }));
    const row = modelRow();
    fireEvent.click(row.getByRole('button'));
    await waitFor(() => expect(row.getByText('GPT-5 Nano')).toBeInTheDocument());
    fireEvent.click(row.getByText('GPT-5 Nano'));
    await waitFor(() => expect(save).toHaveBeenCalled());
    const patched = save.mock.calls.at(-1)![0] as {
      agents: { autoTitle: { models: Record<string, string> } };
    };
    expect(patched.agents.autoTitle.models).toEqual({ claude: 'haiku', codex: 'gpt-5-nano' });
  });

  it('reads back each harness’s own entry', async () => {
    renderSection({
      agents: { autoTitle: { models: { claude: 'haiku', codex: 'gpt-5-nano' } } },
    });
    await waitFor(() => expect(modelRow().getByRole('button').textContent).toMatch(/haiku/i));
    const harnesses = within(screen.getByText('Title model for').closest('div') as HTMLElement);
    fireEvent.click(harnesses.getByRole('button', { name: 'Codex' }));
    await waitFor(() => expect(modelRow().getByRole('button').textContent).toMatch(/gpt-5-nano/i));
  });
});
