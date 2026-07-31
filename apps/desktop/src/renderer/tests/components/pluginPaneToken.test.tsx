/**
 * A plugin pane's bus token is now attached locally, on mount, rather than
 * being carried in the pane URL that gets published.
 *
 * The layout document the hub persists (0644) and mirrors to every client used
 * to contain the static per-plugin token, because it was baked into pane.url.
 * useLayoutSync redacts it on the way out, which means a pane restored from a
 * shared layout arrives with no credential at all — so every pane that knows
 * its plugin has to mint its own, not just the agent-scoped ones that were
 * doing it for confinement.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import React from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import PluginPane from '../../src/panes/PluginPane';

vi.mock('../../src/panes/BrowserPane', () => ({
  default: ({ initialUrl }: { initialUrl: string }) => (
    <div data-testid="browser" data-url={initialUrl} />
  ),
}));

const PLAIN_URL = 'http://127.0.0.1:9999/index.html?sessionId=abc';

let mint: ReturnType<typeof vi.fn>;
let revoke: ReturnType<typeof vi.fn>;

beforeEach(() => {
  mint = vi.fn().mockResolvedValue('minted-token');
  revoke = vi.fn();
  (window as any).electronAPI = {
    ...(window as any).electronAPI,
    pluginPaneToken: mint,
    revokePluginPaneToken: revoke,
  };
});

const url = () => screen.getByTestId('browser').getAttribute('data-url');

describe('PluginPane bus token', () => {
  it('mints a token for a global pane (no cwd) and attaches it to the URL', async () => {
    render(
      <PluginPane paneId="p1" title="Plugin" isActive url={PLAIN_URL} pluginId="acme.widget" />,
    );
    await waitFor(() => expect(screen.queryByTestId('browser')).not.toBeNull());
    expect(mint).toHaveBeenCalledWith('acme.widget', undefined);
    expect(url()).toContain('busToken=minted-token');
    expect(url()).toContain('sessionId=abc');
  });

  it('binds the agent cwd when the pane has one', async () => {
    render(
      <PluginPane
        paneId="p1"
        title="Plugin"
        isActive
        url={PLAIN_URL}
        pluginId="acme.widget"
        cwd="/repo"
      />,
    );
    await waitFor(() => expect(screen.queryByTestId('browser')).not.toBeNull());
    expect(mint).toHaveBeenCalledWith('acme.widget', '/repo');
  });

  it('revokes the minted token when the pane unmounts', async () => {
    const { unmount } = render(
      <PluginPane paneId="p1" title="Plugin" isActive url={PLAIN_URL} pluginId="acme.widget" />,
    );
    await waitFor(() => expect(url()).toContain('busToken=minted-token'));
    unmount();
    expect(revoke).toHaveBeenCalledWith('minted-token');
  });

  it('falls back to the URL as given when minting is unavailable', () => {
    (window as any).electronAPI.pluginPaneToken = undefined;
    render(
      <PluginPane paneId="p1" title="Plugin" isActive url={PLAIN_URL} pluginId="acme.widget" />,
    );
    expect(url()).toBe(PLAIN_URL);
  });

  it('falls back to the URL as given when the mint fails', async () => {
    mint.mockResolvedValue(null);
    render(
      <PluginPane paneId="p1" title="Plugin" isActive url={PLAIN_URL} pluginId="acme.widget" />,
    );
    await waitFor(() => expect(screen.queryByTestId('browser')).not.toBeNull());
    expect(url()).toBe(PLAIN_URL);
  });
});
