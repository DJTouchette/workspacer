/**
 * WORKSPACER_PLUGIN_SANDBOX=enforce on a host with no confinement mechanism
 * (Linux without bubblewrap) is a PERMANENT refusal: the hub never constructs a
 * supervisor for that plugin, so no `sidecar.running` / `sidecar.crashed` event
 * is ever emitted — and this pane derives its state from those events alone,
 * falling back to the optimistic "starting". The result was an in-progress
 * label, forever, for a process that will never be started.
 *
 * `plugin.sandbox.refused` is the only signal that exists for that outcome and
 * no renderer consumed it (grep found it only as a permission pattern).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import PluginsManagerPane from '../../src/panes/PluginsManagerPane';

const PLUGIN = {
  id: 'demo',
  name: 'Demo',
  apiVersion: '1',
  server: { command: './server' },
} as never;

let emit: ((ev: { type: string; data?: unknown }) => void) | null;

vi.mock('../../src/hooks/usePlugins', () => ({
  usePlugins: () => ({ plugins: [PLUGIN], reload: vi.fn() }),
}));

beforeEach(() => {
  emit = null;
  (window as never as { electronAPI: Record<string, unknown> }).electronAPI = {
    onHubEvent: (cb: (ev: { type: string; data?: unknown }) => void) => {
      emit = cb;
      return () => {};
    },
    checkPluginUpdates: vi.fn().mockResolvedValue({}),
    listExamplePlugins: vi.fn().mockResolvedValue([]),
  };
});

describe('PluginsManagerPane — a sidecar the sandbox refused', () => {
  it('does not show a permanent "starting" for a plugin that will never start', async () => {
    render(<PluginsManagerPane />);
    // Before any event: the optimistic default, which is correct while the hub
    // is genuinely still bringing it up.
    expect(screen.getByText('starting')).toBeTruthy();

    await act(async () => {
      emit!({
        type: 'plugin.sandbox.refused',
        data: { id: 'demo', reason: 'no confinement mechanism available' },
      });
    });

    expect(screen.queryByText('starting'), 'a refusal is permanent, not in progress').toBeNull();
    expect(screen.getByText('refused')).toBeTruthy();
    expect(screen.getByText(/no confinement mechanism available/)).toBeTruthy();
  });
});
