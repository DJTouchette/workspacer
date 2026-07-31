import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';
import React from 'react';
import { ConfigProvider } from '../src/contexts/ConfigContext';
import { useConfig } from '../src/hooks/useConfig';

/**
 * The save seam, end to end. Two guarantees:
 *
 *  1. A save carries only what it changed. Callers spread a whole subtree
 *     (`{ ui: { ...config.ui, sidebarWidth } }`) from a snapshot that may be
 *     minutes old, and main replaces `ui.customThemes` wholesale — so a spread
 *     used to be able to delete a theme created on another client.
 *  2. The snapshot re-syncs when main says the config changed, so the window in
 *     which it can be stale stays small.
 */

const saveConfig = vi.fn();
const getConfig = vi.fn();
let pushConfig: ((cfg: unknown) => void) | undefined;

const BOOT = {
  ui: { theme: 'everforest', sidebarWidth: 296, customThemes: { 'custom:a': { name: 'A' } } },
  claude: { keepWarm: false, seenModels: [] as string[] },
};

beforeEach(() => {
  // Module-scoped spies: clear the call log, not just the implementation.
  saveConfig.mockReset();
  getConfig.mockReset();
  pushConfig = undefined;
  getConfig.mockResolvedValue(structuredClone(BOOT));
  saveConfig.mockImplementation((partial: any) => Promise.resolve({ ...BOOT, ...partial }));
  (window.electronAPI as any).getConfig = getConfig;
  (window.electronAPI as any).saveConfig = saveConfig;
  (window.electronAPI as any).onConfigChanged = (cb: (cfg: unknown) => void) => {
    pushConfig = cb;
    return () => {
      pushConfig = undefined;
    };
  };
});

/** Renders the config and exposes save() through a button. */
const Probe: React.FC<{ patch: Record<string, unknown> }> = ({ patch }) => {
  const { config, save } = useConfig();
  return (
    <div>
      <span data-testid="width">{String(config.ui?.sidebarWidth)}</span>
      <span data-testid="themes">{Object.keys(config.ui?.customThemes ?? {}).join(',')}</span>
      <button onClick={() => void save(patch as never)}>save</button>
    </div>
  );
};

const mount = (patch: Record<string, unknown>) =>
  render(
    <ConfigProvider>
      <Probe patch={patch} />
    </ConfigProvider>,
  );

describe('ConfigProvider.save', () => {
  it('sends only the leaf that changed, not the whole spread subtree', async () => {
    mount({ ui: { ...BOOT.ui, sidebarWidth: 340 } });
    await waitFor(() => expect(screen.getByTestId('width').textContent).toBe('296'));
    screen.getByText('save').click();
    await waitFor(() => expect(saveConfig).toHaveBeenCalled());
    expect(saveConfig).toHaveBeenCalledWith({ ui: { sidebarWidth: 340 } });
  });

  it('does not carry a stale customThemes along for the ride', async () => {
    mount({ ui: { ...BOOT.ui, sidebarWidth: 340 } });
    await waitFor(() => expect(screen.getByTestId('width').textContent).toBe('296'));
    screen.getByText('save').click();
    await waitFor(() => expect(saveConfig).toHaveBeenCalled());
    // This is the data loss: main treats a supplied customThemes as the whole
    // truth, so shipping the boot copy would delete anything added since.
    expect(JSON.stringify(saveConfig.mock.calls[0][0])).not.toContain('customThemes');
  });

  it('still sends customThemes when the caller is genuinely editing it', async () => {
    mount({ ui: { ...BOOT.ui, customThemes: {} } });
    await waitFor(() => expect(screen.getByTestId('themes').textContent).toBe('custom:a'));
    screen.getByText('save').click();
    await waitFor(() => expect(saveConfig).toHaveBeenCalled());
    expect(saveConfig).toHaveBeenCalledWith({ ui: { customThemes: {} } });
  });

  it('skips the IPC entirely when nothing changed', async () => {
    mount({ ui: { ...BOOT.ui } });
    await waitFor(() => expect(screen.getByTestId('width').textContent).toBe('296'));
    screen.getByText('save').click();
    await new Promise((r) => setTimeout(r, 10));
    expect(saveConfig).not.toHaveBeenCalled();
  });

  it('re-syncs from a main-process push, so the next save diffs against truth', async () => {
    mount({ ui: { ...BOOT.ui, sidebarWidth: 340 } });
    await waitFor(() => expect(screen.getByTestId('width').textContent).toBe('296'));

    // Main writes config behind our back — here, a theme created on the phone.
    act(() => {
      pushConfig?.({
        ...BOOT,
        ui: { ...BOOT.ui, customThemes: { 'custom:a': { name: 'A' }, 'custom:b': { name: 'B' } } },
      });
    });
    await waitFor(() => expect(screen.getByTestId('themes').textContent).toBe('custom:a,custom:b'));
  });
});
