import React, { createContext, useCallback, useContext, useEffect, useState } from 'react';
import type { Config } from '../hooks/useConfig';
import { DEFAULT_CONFIG } from '../hooks/useConfig';

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

  // Initial load on mount.
  useEffect(() => {
    window.electronAPI.getConfig()
      .then((cfg) => {
        setConfig(cfg as Config);
        setLoaded(true);
      })
      .catch(() => {
        setLoaded(true);
      });
  }, []);

  const reload = useCallback(() => {
    window.electronAPI.reloadConfig()
      .then((cfg) => {
        setConfig(cfg as Config);
      })
      .catch(console.error);
  }, []);

  const save = useCallback((partial: Partial<Config>): Promise<Config> => {
    return window.electronAPI.saveConfig(partial)
      .then((cfg) => {
        setConfig(cfg as Config);
        return cfg as Config;
      });
  }, []);

  const value: ConfigContextValue = { config, loaded, reload, save };

  return (
    <ConfigContext.Provider value={value}>
      {children}
    </ConfigContext.Provider>
  );
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
