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

vi.mock('../src/lib/notificationBus', () => ({ postNotification: vi.fn() }));

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

/**
 * Wait for the BOOT config to actually arrive.
 *
 * NOT on sidebarWidth: DEFAULT_CONFIG ships the same 296 (and the same
 * 'everforest' theme), so `waitFor(width === '296')` is satisfied by the
 * pre-load render and waits for nothing at all. Every test in this file turns
 * on the difference between the defaults and the loaded config, so the wait has
 * to be able to tell them apart — customThemes is the one field only the loaded
 * config has.
 *
 * Observed, not theorised: with the width wait, a full-suite run had
 * `getConfig()` still unresolved at click time, so the patch was computed
 * against DEFAULT_CONFIG and carried customThemes — failing the first case
 * intermittently and, worse, making the customThemes cases below pass on runs
 * where they never exercised their own subject.
 */
const awaitBoot = () =>
  waitFor(() => expect(screen.getByTestId('themes').textContent).toBe('custom:a'));

describe('ConfigProvider.save', () => {
  it('sends only the leaf that changed, not the whole spread subtree', async () => {
    mount({ ui: { ...BOOT.ui, sidebarWidth: 340 } });
    await awaitBoot();
    screen.getByText('save').click();
    await waitFor(() => expect(saveConfig).toHaveBeenCalled());
    expect(saveConfig).toHaveBeenCalledWith({ ui: { sidebarWidth: 340 } });
  });

  it('does not carry a stale customThemes along for the ride', async () => {
    mount({ ui: { ...BOOT.ui, sidebarWidth: 340 } });
    await awaitBoot();
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
    await awaitBoot();
    screen.getByText('save').click();
    await new Promise((r) => setTimeout(r, 10));
    expect(saveConfig).not.toHaveBeenCalled();
  });

  it('re-syncs from a main-process push, so the next save diffs against truth', async () => {
    mount({ ui: { ...BOOT.ui, sidebarWidth: 340 } });
    await awaitBoot();

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

// ─── the failure plane ───────────────────────────────────────────────────────
// The known specimen for "a catch that converts a failure into a default":
// getConfig().catch() ran the whole app on DEFAULT_CONFIG with loaded=true, and
// save()'s rejection was unhandled — so a refused save left the UI showing the
// value the user picked, as if it had been applied, until the next read
// reverted it.

describe('ConfigProvider — failures are not silently converted to defaults', () => {
  it('tells the user when the config could not be loaded at all', async () => {
    const notify = vi.mocked((await import('../src/lib/notificationBus')).postNotification);
    notify.mockClear();
    getConfig.mockRejectedValue(new Error('bus: no provider for config.get'));

    mount({});
    await waitFor(() => expect(notify).toHaveBeenCalled());
    const [n] = notify.mock.calls.at(-1)!;
    expect(n.title).toBe('Settings could not be loaded');
    expect(String(n.body)).toContain('no provider for config.get');
    expect(n.level).toBe('warn');
  });

  it('keeps the previous snapshot and says so when a save is refused', async () => {
    const notify = vi.mocked((await import('../src/lib/notificationBus')).postNotification);
    notify.mockClear();
    saveConfig.mockRejectedValue(new Error('config.yaml is locked by another process'));

    mount({ ui: { ...BOOT.ui, sidebarWidth: 340 } });
    await awaitBoot();
    await act(async () => {
      screen.getByText('save').click();
    });

    await waitFor(() => expect(notify).toHaveBeenCalled());
    const [n] = notify.mock.calls.at(-1)!;
    expect(n.title).toBe('Setting not saved');
    expect(String(n.body)).toContain('locked by another process');
    // …and nothing painted the refused value as applied.
    expect(screen.getByTestId('width').textContent).toBe('296');
  });
});
