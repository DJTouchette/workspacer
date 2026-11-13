/**
 * v0.170.0 is the last Electron release. The desktop shell has to say so on
 * launch and in Settings → Updates, and link to the native app's releases
 * page in the default browser (never by navigating the webview).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import SystemNotices from '../src/components/SystemNotices';
import UpdatesSection from '../src/components/settings/UpdatesSection';
import type { Config } from '../src/hooks/useConfig';
import { NATIVE_DOWNLOAD_URL } from '../src/lib/nativeMove';

const api = () => window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

beforeEach(() => {
  api().updatesGetStatus = vi.fn().mockResolvedValue({ state: 'idle', current: '0.170.0' });
  api().openExternalUrl = vi.fn().mockResolvedValue({ ok: true });
  api().onSystemNotice = vi.fn(() => () => {});
});

describe('launch notice — Workspacer moved to native', () => {
  it('shows on launch and opens the releases page externally', async () => {
    render(<SystemNotices />);
    expect(await screen.findByText('Workspacer has moved to a native app')).toBeInTheDocument();
    expect(screen.getByText(/final Electron release/)).toBeInTheDocument();
    // Not a failure notice: no "Open logs".
    expect(screen.queryByText('Open logs')).not.toBeInTheDocument();

    fireEvent.click(screen.getByText('Download Workspacer Native'));
    expect(api().openExternalUrl).toHaveBeenCalledWith(NATIVE_DOWNLOAD_URL);
  });

  it('dismisses for the session', async () => {
    render(<SystemNotices />);
    await screen.findByText('Workspacer has moved to a native app');
    fireEvent.click(screen.getByLabelText('Dismiss'));
    expect(screen.queryByText('Workspacer has moved to a native app')).not.toBeInTheDocument();
  });

  it('stays out of the web client, which reports no app version', async () => {
    api().updatesGetStatus = vi.fn().mockResolvedValue({ state: 'unsupported', current: '' });
    render(<SystemNotices />);
    await waitFor(() => expect(api().updatesGetStatus).toHaveBeenCalled());
    expect(screen.queryByText('Workspacer has moved to a native app')).not.toBeInTheDocument();
  });
});

describe('Settings → Updates — final Electron release', () => {
  it('says no further updates will come and links to the native app', async () => {
    render(<UpdatesSection config={{} as Config} save={vi.fn()} />);
    expect(
      await screen.findByText(/No further updates will come to this Electron app/),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByText('Download Workspacer Native'));
    expect(api().openExternalUrl).toHaveBeenCalledWith(NATIVE_DOWNLOAD_URL);
  });
});
