import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, within, waitFor } from '@testing-library/react';
import React from 'react';
import StatusSummarySettings from '../../src/components/settings/StatusSummarySettings';
import type { Config } from '../../src/hooks/useConfig';
import { DEFAULT_CONFIG } from '../../src/hooks/configDefaults';
const state = vi.hoisted(() => ({
  detection: null as null | Array<{
    provider: string;
    found: boolean;
    resolvedPath: null;
    customBin: string;
  }>,
}));
vi.mock('../../src/hooks/useProviderDetection', () => ({
  useProviderDetection: () => ({ detection: state.detection, refresh: vi.fn() }),
}));
beforeEach(() => {
  state.detection = null;
  window.electronAPI.claudeListModels = vi.fn().mockResolvedValue({
    aliases: [{ value: 'haiku', label: 'Haiku' }],
    seen: [],
    defaultModel: '',
  });
  window.electronAPI.providerListModels = vi
    .fn()
    .mockResolvedValue([{ id: 'gpt-5-nano', label: 'GPT-5 Nano' }]);
});
function setup(summary = DEFAULT_CONFIG.agents.statusSummary) {
  const save = vi.fn().mockResolvedValue({});
  render(
    <StatusSummarySettings
      config={
        {
          ...DEFAULT_CONFIG,
          agents: { ...DEFAULT_CONFIG.agents, statusSummary: summary },
        } as Config
      }
      save={save}
    />,
  );
  return save;
}
describe('status summary settings', () => {
  it('ships enabled and persists claude with an Anthropic label', async () => {
    const save = setup();
    expect(screen.getByRole('checkbox')).toBeChecked();
    fireEvent.click(screen.getByRole('button', { name: 'Anthropic / Claude' }));
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        agents: expect.objectContaining({
          statusSummary: { enabled: true, provider: 'claude', model: null },
        }),
      }),
    );
  });
  it('resets incompatible model when switching provider and preserves siblings', () => {
    const save = setup();
    fireEvent.click(screen.getByRole('button', { name: 'Codex', exact: true }));
    const config = save.mock.calls[0][0] as Config;
    expect(config.agents.statusSummary).toEqual({ enabled: true, provider: 'codex', model: null });
    expect(config.agents.autoTitle).toEqual(DEFAULT_CONFIG.agents.autoTitle);
  });
  it('keeps an incompatible configured value visible and promises no fallback', async () => {
    setup({ enabled: true, provider: 'codex', model: 'haiku' });
    await waitFor(() =>
      expect(
        screen.getByText(/belongs to a different harness and will be refused/),
      ).toBeInTheDocument(),
    );
    expect(screen.getByText(/no provider or model fallback/)).toBeInTheDocument();
    expect(screen.queryByText(/spawn drops it/)).toBeNull();
  });
  it('offers only auto and default for Copilot', () => {
    setup({ enabled: true, provider: 'copilot', model: null });
    const row = within(screen.getByText('Status summary model').closest('div')!);
    fireEvent.click(row.getByRole('button'));
    expect(row.getByText('Auto')).toBeInTheDocument();
    expect(screen.queryByText('GPT-5 Nano')).toBeNull();
  });
  it('disables without changing selected provider/model', () => {
    const save = setup();
    fireEvent.click(screen.getByRole('checkbox'));
    expect(save.mock.calls[0][0].agents.statusSummary).toEqual({
      enabled: false,
      provider: 'claude',
      model: 'haiku',
    });
  });
});

it('keeps a configured missing provider visible and flagged', () => {
  state.detection = [{ provider: 'claude', found: false, resolvedPath: null, customBin: '' }];
  setup();
  expect(
    screen.getByRole('button', { name: 'Anthropic / Claude (not installed)' }),
  ).toBeInTheDocument();
});
