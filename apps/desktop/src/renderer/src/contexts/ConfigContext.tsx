import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import type { Config } from '../hooks/useConfig';
// Import DEFAULT_CONFIG from the leaf module (NOT ../hooks/useConfig) so this
// context isn't in an import cycle with the useConfig hook — the cycle could
// duplicate this module under Vite HMR and break lazy-loaded panes with
// "useConfig must be used inside <ConfigProvider>".
import { DEFAULT_CONFIG } from '../hooks/configDefaults';
import { minimalConfigPatch } from '../lib/configPatch';

// ---------------------------------------------------------------------------
// Context shape — mirrors the useConfig return value exactly.
// ---------------------------------------------------------------------------
export interface ConfigContextValue {
  config: Config;
  loaded: boolean;
  reload: () => void;
  save: (partial: Partial<Config>) => Promise<Config>;
}

export const ConfigContext = createContext<ConfigContextValue | null>(null);

// ---------------------------------------------------------------------------
// Provider — owns config state and all IPC calls.
// ---------------------------------------------------------------------------
export function ConfigProvider({ children }: { children: React.ReactNode }) {
  const [config, setConfig] = useState<Config>(DEFAULT_CONFIG);
  const [loaded, setLoaded] = useState(false);
  // The latest snapshot, readable without making `save` depend on it (a save
  // that re-binds on every config change would re-render every consumer).
  const configRef = useRef(config);
  configRef.current = config;

  // Initial load on mount.
  useEffect(() => {
    window.electronAPI
      .getConfig()
      .then((cfg) => {
        setConfig(cfg as Config);
        setLoaded(true);
      })
      .catch(() => {
        setLoaded(true);
      });
  }, []);

  // Config is written by more than this renderer: main writes it (an agent's
  // seen models, budgets), and so does the brain for the web/phone clients.
  // Without this the snapshot only ever refreshed on our own save, so Settings
  // could show — and re-send — values that were replaced hours ago.
  useEffect(() => {
    return window.electronAPI.onConfigChanged?.((cfg) => setConfig(cfg as Config));
  }, []);

  const reload = useCallback(() => {
    window.electronAPI
      .reloadConfig()
      .then((cfg) => {
        setConfig(cfg as Config);
      })
      .catch(console.error);
  }, []);

  // Every save is trimmed to what actually changed before it leaves the
  // renderer. Callers write `save({ ui: { ...config.ui, x } })`, which re-sends
  // every sibling from THIS snapshot — and anything written behind our back
  // since (an agent's seenModels, a theme created on the phone) would be
  // overwritten by our older copy. See lib/configPatch.
  const save = useCallback((partial: Partial<Config>): Promise<Config> => {
    const patch = minimalConfigPatch(
      configRef.current as unknown as Record<string, unknown>,
      partial,
    );
    if (Object.keys(patch).length === 0) return Promise.resolve(configRef.current);
    return window.electronAPI.saveConfig(patch).then((cfg) => {
      setConfig(cfg as Config);
      return cfg as Config;
    });
  }, []);

  const value: ConfigContextValue = { config, loaded, reload, save };

  return <ConfigContext.Provider value={value}>{children}</ConfigContext.Provider>;
}

// ---------------------------------------------------------------------------
// Internal hook — used by useConfig.ts.
// ---------------------------------------------------------------------------
export function useConfigContext(): ConfigContextValue {
  const ctx = useContext(ConfigContext);
  if (ctx === null) {
    throw new Error('useConfig must be used inside <ConfigProvider>');
  }
  return ctx;
}
