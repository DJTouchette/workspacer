/**
 * Keep-warm's "Warm providers" buttons, which used to be a hardcoded
 * `['claude','codex']` while every other picker in the same panel went through
 * `visibleProviderOptions` — so a Copilot-only machine was offered two
 * harnesses it does not have.
 *
 * The rule these pin is an INTERSECTION, and both halves matter:
 *
 *  - keep-warm supports claude and codex ONLY, and that is correct rather than
 *    a gap: they are the only harnesses with a 5-hour subscription window a
 *    ping can start (both sides read `main/shared/keepWarmProviders`, so the
 *    service filters the config through the very list these buttons offer).
 *    Installing copilot must not add a button — it would write config the
 *    service then drops on the floor.
 *  - of those two, offer what is installed — with the usual escape hatch that a
 *    provider the config is already warming stays visible, flagged, so it can
 *    still be switched off.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within, waitFor } from '@testing-library/react';
import React from 'react';
import SessionSection from '../../src/components/settings/SessionSection';
import { __resetProviderDetectionCache } from '../../src/hooks/useProviderDetection';
import type { Config } from '../../src/hooks/useConfig';

const api = window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

const detection = (rows: Array<[string, boolean]>) =>
  rows.map(([provider, found]) => ({
    provider,
    found,
    resolvedPath: found ? `/usr/bin/${provider}` : null,
    customBin: '',
  }));

const CLAUDE_ONLY = detection([
  ['claude', true],
  ['codex', false],
  ['copilot', false],
  ['opencode', false],
  ['pi', false],
]);

const COPILOT_ONLY = detection([
  ['claude', false],
  ['codex', false],
  ['copilot', true],
  ['opencode', false],
  ['pi', false],
]);

const ALL_FIVE = detection([
  ['claude', true],
  ['codex', true],
  ['copilot', true],
  ['opencode', true],
  ['pi', true],
]);

beforeEach(() => {
  __resetProviderDetectionCache();
  api.claudeListModels = vi.fn().mockResolvedValue({ aliases: [], seen: [], defaultModel: '' });
  api.providerListModels = vi.fn().mockResolvedValue([]);
  api.keepWarmHeartbeats = vi.fn().mockResolvedValue([]);
});

function renderKeepWarm(providers: string[]) {
  const config = {
    claude: {
      keepWarm: {
        enabled: true,
        providers,
        mode: 'auto' as const,
        intervalHours: 5,
        dailyAt: '08:00',
      },
    },
  };
  return render(<SessionSection config={config as Config} save={vi.fn().mockResolvedValue({})} />);
}

/** The buttons inside the "Warm providers" row, once it exists. */
const warmRow = async () => {
  const label = await screen.findByText('Warm providers');
  return within(label.closest('div') as HTMLElement);
};

describe('SessionSection — the keep-warm provider buttons', () => {
  it('drops a warmable harness this machine does not have', async () => {
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_ONLY);
    renderKeepWarm(['claude']);

    await waitFor(async () =>
      expect((await warmRow()).queryByRole('button', { name: /Codex/ })).toBeNull(),
    );
    expect((await warmRow()).getByRole('button', { name: 'Claude' })).toBeTruthy();
  });

  it('never offers a harness with no 5-hour window, even when it IS installed', async () => {
    // The whole point: this list is the intersection, not "whatever is
    // installed". Copilot has no readable quota and opencode/pi are
    // bring-your-own-key — a ping would buy nothing.
    api.providerCheckAll = vi.fn().mockResolvedValue(ALL_FIVE);
    renderKeepWarm(['claude']);

    const row = await warmRow();
    await waitFor(() => expect(row.getByRole('button', { name: 'Codex' })).toBeTruthy());
    for (const name of [/Copilot/, /OpenCode/, /^Pi/]) {
      expect(row.queryByRole('button', { name })).toBeNull();
    }
  });

  it('keeps a configured-but-missing warm target visible and flagged', async () => {
    // config says "warm claude" and the service will keep trying; hiding the
    // button would leave no way to turn that off.
    api.providerCheckAll = vi.fn().mockResolvedValue(COPILOT_ONLY);
    renderKeepWarm(['claude']);

    const row = await warmRow();
    await waitFor(() =>
      expect(row.getByRole('button', { name: /Claude \(not installed\)/ })).toBeTruthy(),
    );
  });

  it('explains itself instead of rendering an empty row when nothing is warmable', async () => {
    api.providerCheckAll = vi.fn().mockResolvedValue(COPILOT_ONLY);
    renderKeepWarm([]);

    await waitFor(() => expect(screen.getByText(/Nothing to warm on this machine/)).toBeTruthy());
    expect(screen.queryByText('Warm providers')).toBeNull();
  });
});
