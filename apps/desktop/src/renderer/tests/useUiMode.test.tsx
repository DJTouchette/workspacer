/**
 * useUiMode — the seam between config.ui.mode and the mode manifest.
 * Verifies the fleet default and that toggle() round-trips through the
 * config save path (the same IPC the rest of the app persists through).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import React from 'react';

import { useUiMode } from '../src/hooks/useUiMode';
import { MODE_MANIFEST } from '../src/lib/uiMode';
import { ConfigProvider } from '../src/contexts/ConfigContext';
import { DEFAULT_CONFIG } from '../src/hooks/configDefaults';

const wrapper = ({ children }: { children: React.ReactNode }) =>
  React.createElement(ConfigProvider, null, children);

describe('useUiMode', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    window.electronAPI.getConfig = vi
      .fn()
      .mockResolvedValue({ ...DEFAULT_CONFIG, ui: { ...DEFAULT_CONFIG.ui, mode: undefined } });
    // Echo main's deep-merge: the saved partial's ui block wins over the base.
    window.electronAPI.saveConfig = vi.fn().mockImplementation(async (partial: any) => ({
      ...DEFAULT_CONFIG,
      ...partial,
      ui: { ...DEFAULT_CONFIG.ui, ...(partial.ui ?? {}) },
    }));
  });

  it('defaults to fleet with the fleet manifest', async () => {
    const { result } = renderHook(() => useUiMode(), { wrapper });
    await waitFor(() => expect(result.current.mode).toBe('fleet'));
    expect(result.current.manifest).toBe(MODE_MANIFEST.fleet);
  });

  it('toggle() round-trips the mode through config save', async () => {
    const { result } = renderHook(() => useUiMode(), { wrapper });
    await waitFor(() => expect(result.current.mode).toBe('fleet'));

    await act(async () => {
      result.current.toggle();
    });
    expect(window.electronAPI.saveConfig).toHaveBeenCalledWith(
      expect.objectContaining({ ui: expect.objectContaining({ mode: 'focus' }) }),
    );
    await waitFor(() => expect(result.current.mode).toBe('focus'));
    expect(result.current.manifest).toBe(MODE_MANIFEST.focus);
    expect(result.current.manifest.fleetDeck).toBe(false);
    expect(result.current.manifest.inspectorRail).toBe(false);

    await act(async () => {
      result.current.toggle();
    });
    expect(window.electronAPI.saveConfig).toHaveBeenLastCalledWith(
      expect.objectContaining({ ui: expect.objectContaining({ mode: 'fleet' }) }),
    );
    await waitFor(() => expect(result.current.mode).toBe('fleet'));
    expect(result.current.manifest).toBe(MODE_MANIFEST.fleet);
  });

  it('setMode() writes the requested mode', async () => {
    const { result } = renderHook(() => useUiMode(), { wrapper });
    await waitFor(() => expect(result.current.mode).toBe('fleet'));

    await act(async () => {
      result.current.setMode('focus');
    });
    await waitFor(() => expect(result.current.mode).toBe('focus'));

    // Idempotent: setting the current mode again keeps it.
    await act(async () => {
      result.current.setMode('focus');
    });
    await waitFor(() => expect(result.current.mode).toBe('focus'));
  });
});
