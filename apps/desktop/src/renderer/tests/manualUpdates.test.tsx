/**
 * The macOS build is unsigned, so electron-updater is gated off there and main
 * reports `state: 'manual'` instead of erroring every four hours (see
 * main/services/updateService.ts). Gating it off silently would only trade a
 * visible failure for an invisible one — a mac user has to be able to tell that
 * updates are theirs to do by hand.
 *
 * These tests pin the two surfaces that carry it: the command palette entry and
 * the Settings → Updates section.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import CommandPalette from '../src/components/CommandPalette';
import UpdatesSection from '../src/components/settings/UpdatesSection';
import { ConfigProvider } from '../src/contexts/ConfigContext';
import type { Config } from '../src/hooks/useConfig';
import type { UpdateStatus } from '../src/types/electron';

const api = () => window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

/** What main's IPC would answer on this platform. */
function stubUpdateStatus(state: UpdateStatus['state']): void {
  api().updatesGetStatus = vi.fn().mockResolvedValue({ state, current: '0.150.0' });
}

describe('command palette — updates on a platform with no updater', () => {
  it('offers the releases page instead of a check that can only fail', () => {
    const onCheckUpdates = vi.fn();
    render(
      <ConfigProvider>
        <CommandPalette
          visible
          apps={[]}
          onClose={vi.fn()}
          onLaunchApp={vi.fn()}
          updateStatus={{ state: 'manual', current: '0.150.0' }}
          onCheckUpdates={onCheckUpdates}
        />
      </ConfigProvider>,
    );

    // Named for what it does there — "Check for Updates" would be a lie, since
    // checkNow() on this platform opens the browser rather than checking.
    expect(screen.getByText('Download the Latest Release')).toBeInTheDocument();
    expect(screen.queryByText('Check for Updates')).not.toBeInTheDocument();

    screen.getByText('Download the Latest Release').click();
    expect(onCheckUpdates).toHaveBeenCalledTimes(1);
  });

  it('still offers a real check on a platform that has an updater', () => {
    render(
      <ConfigProvider>
        <CommandPalette
          visible
          apps={[]}
          onClose={vi.fn()}
          onLaunchApp={vi.fn()}
          updateStatus={{ state: 'idle', current: '0.150.0' }}
          onCheckUpdates={vi.fn()}
        />
      </ConfigProvider>,
    );

    expect(screen.getByText('Check for Updates')).toBeInTheDocument();
    expect(screen.queryByText('Download the Latest Release')).not.toBeInTheDocument();
  });
});

describe('Settings → Updates — the manual-update notice', () => {
  beforeEach(() => {
    stubUpdateStatus('idle');
  });

  it('says updates are manual and stops offering a toggle that does nothing', async () => {
    stubUpdateStatus('manual');
    render(<UpdatesSection config={{} as Config} save={vi.fn()} />);

    expect(await screen.findByText(/Updates are manual on this platform/)).toBeInTheDocument();
    // The auto-update switch has nothing to switch here.
    await waitFor(() =>
      expect(
        screen.getByRole('checkbox', { name: /Automatically check for and install updates/i }),
      ).toBeDisabled(),
    );
  });

  it('keeps the ordinary copy and a live toggle where auto-update works', async () => {
    render(<UpdatesSection config={{} as Config} save={vi.fn()} />);

    expect(await screen.findByText(/Checks the GitHub release feed on launch/)).toBeInTheDocument();
    expect(screen.queryByText(/Updates are manual on this platform/)).not.toBeInTheDocument();
    expect(
      screen.getByRole('checkbox', { name: /Automatically check for and install updates/i }),
    ).not.toBeDisabled();
  });
});
