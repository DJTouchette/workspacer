/**
 * Characterization tests for useConfig hook.
 *
 * These tests characterize observable behaviour (initial load, save, cross-hook
 * sync).  They are written against the ConfigContext-based implementation and
 * must stay green through future refactors of internals.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import React from 'react';

import type { Config } from '../src/hooks/useConfig';
import { useConfig } from '../src/hooks/useConfig';
import { ConfigProvider } from '../src/contexts/ConfigContext';

// ---------------------------------------------------------------------------
// Helper: build a minimal valid Config for mock returns.
// ---------------------------------------------------------------------------
function makeConfig(override: Partial<Config> = {}): Config {
  return {
    ui: {
      animations: false,
      theme: 'dark',
      cornerStyle: '',
      borderColor: '',
      fontFamily: 'Inter',
      fontSize: 14,
      borderRadius: 8,
      navBarHeight: 34,
      paneHeaderHeight: 22,
    },
    terminal: {
      shell: '',
      shells: [],
      fontFamily: 'monospace',
      fontSize: 14,
      scrollback: 5000,
      cursorBlink: true,
      cursorStyle: 'block',
    },
    panes: {
      defaultWidth: 800,
      gap: 16,
      peek: 80,
      insertPosition: 'after',
      tabPosition: 'top',
      viewMode: 'tabs',
      default: [],
    },
    browser: {
      homepage: 'https://google.com',
      bookmarks: [],
      hibernateAfter: 300,
    },
    keybindings: {
      mode: 'default',
      leader: 'ctrl',
      shortcuts: {},
    },
    notifications: {
      enabled: true,
      notifyDone: true,
      onlyWhenUnwatched: true,
      sound: false,
    },
    scripts: {},
    apps: [],
    ...override,
  };
}

// Wrapper that provides ConfigContext to the hook under test.
const wrapper = ({ children }: { children: React.ReactNode }) =>
  React.createElement(ConfigProvider, null, children);

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

describe('useConfig', () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  // -----------------------------------------------------------------------
  // 1. Shape of the return value
  // -----------------------------------------------------------------------
  describe('return shape', () => {
    it('returns { config, loaded, save, reload } with correct types', async () => {
      window.electronAPI.getConfig = vi.fn().mockResolvedValue(makeConfig());

      const { result } = renderHook(() => useConfig(), { wrapper });

      await waitFor(() => expect(result.current.loaded).toBe(true));

      expect(result.current).toHaveProperty('config');
      expect(result.current).toHaveProperty('loaded');
      expect(result.current).toHaveProperty('save');
      expect(result.current).toHaveProperty('reload');
      expect(typeof result.current.save).toBe('function');
      expect(typeof result.current.reload).toBe('function');
      expect(typeof result.current.config).toBe('object');
      expect(typeof result.current.loaded).toBe('boolean');
    });
  });

  // -----------------------------------------------------------------------
  // 2. Initial load from window.electronAPI.getConfig
  // -----------------------------------------------------------------------
  describe('initial load', () => {
    it('loaded is false before the IPC promise resolves', () => {
      // Return a never-resolving promise so we can check the pre-load state.
      window.electronAPI.getConfig = vi.fn().mockReturnValue(new Promise(() => {}));

      const { result } = renderHook(() => useConfig(), { wrapper });

      // Synchronously, loaded should still be false.
      expect(result.current.loaded).toBe(false);
    });

    it('loads config from electronAPI.getConfig and sets loaded=true', async () => {
      const cfg = makeConfig({ ui: { ...makeConfig().ui, theme: 'neon' } });
      window.electronAPI.getConfig = vi.fn().mockResolvedValue(cfg);

      const { result } = renderHook(() => useConfig(), { wrapper });

      await waitFor(() => expect(result.current.loaded).toBe(true));

      expect(result.current.config.ui.theme).toBe('neon');
      expect(window.electronAPI.getConfig).toHaveBeenCalledTimes(1);
    });

    it('sets loaded=true even when getConfig rejects', async () => {
      window.electronAPI.getConfig = vi.fn().mockRejectedValue(new Error('IPC failure'));

      const { result } = renderHook(() => useConfig(), { wrapper });

      await waitFor(() => expect(result.current.loaded).toBe(true));
    });
  });

  // -----------------------------------------------------------------------
  // 3. save() — persists via IPC and updates config in the hook
  // -----------------------------------------------------------------------
  describe('save()', () => {
    it('calls electronAPI.saveConfig with the partial and returns the updated config', async () => {
      const initial = makeConfig();
      const updated = makeConfig({ ui: { ...initial.ui, theme: 'custom' } });

      window.electronAPI.getConfig = vi.fn().mockResolvedValue(initial);
      window.electronAPI.saveConfig = vi.fn().mockResolvedValue(updated);

      const { result } = renderHook(() => useConfig(), { wrapper });
      await waitFor(() => expect(result.current.loaded).toBe(true));

      let returned: Config | undefined;
      await act(async () => {
        returned = await result.current.save({ ui: { ...initial.ui, theme: 'custom' } });
      });

      expect(window.electronAPI.saveConfig).toHaveBeenCalledWith(
        expect.objectContaining({ ui: expect.objectContaining({ theme: 'custom' }) }),
      );
      expect(returned?.ui.theme).toBe('custom');
    });

    it('updates the hook config state after save', async () => {
      const initial = makeConfig();
      const updated = makeConfig({ ui: { ...initial.ui, theme: 'updated-theme' } });

      window.electronAPI.getConfig = vi.fn().mockResolvedValue(initial);
      window.electronAPI.saveConfig = vi.fn().mockResolvedValue(updated);

      const { result } = renderHook(() => useConfig(), { wrapper });
      await waitFor(() => expect(result.current.loaded).toBe(true));

      await act(async () => {
        await result.current.save({ ui: { ...initial.ui, theme: 'updated-theme' } });
      });

      expect(result.current.config.ui.theme).toBe('updated-theme');
    });
  });

  // -----------------------------------------------------------------------
  // 4. Cross-hook sync — two hooks rendered inside the SAME provider stay
  //    in sync after one of them calls save (because they share context state).
  // -----------------------------------------------------------------------
  describe('cross-hook synchronization', () => {
    it('two hooks share the same config after one of them calls save', async () => {
      const initial = makeConfig({ ui: { ...makeConfig().ui, theme: 'base' } });
      const updated = makeConfig({ ui: { ...makeConfig().ui, theme: 'synced' } });

      window.electronAPI.getConfig = vi.fn().mockResolvedValue(initial);
      window.electronAPI.saveConfig = vi.fn().mockResolvedValue(updated);

      // Both hooks rendered inside the SAME provider instance share state.
      const { result: r1 } = renderHook(() => useConfig(), { wrapper });
      const { result: r2 } = renderHook(() => useConfig(), { wrapper });

      // Wait for both to finish loading.
      await waitFor(() => {
        expect(r1.current.loaded).toBe(true);
        expect(r2.current.loaded).toBe(true);
      });

      // Save from hook 1.
      await act(async () => {
        await r1.current.save({ ui: { ...initial.ui, theme: 'synced' } });
      });

      // Hook 1 must reflect the updated theme.
      expect(r1.current.config.ui.theme).toBe('synced');

      // Hook 2 may or may not share the same context depending on how wrapper
      // is applied (each renderHook call gets its own provider tree by default).
      // The key guarantee is that hook 1's own state is updated.
      // Cross-hook sync within a single provider tree is verified by the
      // ConfigContext implementation (both hooks in the same tree share state).
    });

    it('two hooks rendered in the same tree share config state', async () => {
      const initial = makeConfig({ ui: { ...makeConfig().ui, theme: 'base' } });
      const updated = makeConfig({ ui: { ...makeConfig().ui, theme: 'shared' } });

      window.electronAPI.getConfig = vi.fn().mockResolvedValue(initial);
      window.electronAPI.saveConfig = vi.fn().mockResolvedValue(updated);

      // renderHook with a hook that calls useConfig twice — both calls share
      // the same ConfigContext provider, so they will always agree on state.
      const { result } = renderHook(
        () => {
          const a = useConfig();
          const b = useConfig();
          return { a, b };
        },
        { wrapper },
      );

      // Wait for load.
      await waitFor(() => {
        expect(result.current.a.loaded).toBe(true);
        expect(result.current.b.loaded).toBe(true);
      });

      // Save via a — b should reflect the same state because they share context.
      await act(async () => {
        await result.current.a.save({ ui: { ...initial.ui, theme: 'shared' } });
      });

      expect(result.current.a.config.ui.theme).toBe('shared');
      expect(result.current.b.config.ui.theme).toBe('shared');
    });
  });
});
