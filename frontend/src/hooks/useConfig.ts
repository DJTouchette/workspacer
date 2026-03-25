import { useState, useEffect } from 'react';
import { GetConfig, ReloadConfig } from '../../bindings/workspacer/configservice';
import type { Config } from '../../bindings/workspacer/models';

export type { Config };
export type { UIConfig, TerminalConfig, PanesConfig } from '../../bindings/workspacer/models';

const DEFAULT_CONFIG: Config = {
  ui: {
    animations: false,
    theme: 'dark',
    fontFamily: 'Inter, system-ui, sans-serif',
    fontSize: 14,
    borderRadius: 8,
    navBarHeight: 28,
    paneHeaderHeight: 22,
  },
  terminal: {
    shell: '',
    fontFamily: 'JetBrainsMono NF, JetBrainsMono Nerd Font, CaskaydiaMono NF, monospace',
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
    default: [],
  },
} as Config;

let cachedConfig: Config | null = null;

export function useConfig() {
  const [config, setConfig] = useState<Config>(cachedConfig ?? DEFAULT_CONFIG);
  const [loaded, setLoaded] = useState(cachedConfig !== null);

  useEffect(() => {
    if (cachedConfig) return;
    GetConfig()
      .then((cfg) => {
        cachedConfig = cfg;
        setConfig(cfg);
        setLoaded(true);
      })
      .catch(() => {
        setLoaded(true);
      });
  }, []);

  const reload = () => {
    ReloadConfig()
      .then((cfg) => {
        cachedConfig = cfg;
        setConfig(cfg);
      })
      .catch(console.error);
  };

  return { config, loaded, reload };
}
