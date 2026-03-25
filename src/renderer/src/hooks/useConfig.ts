import { useState, useEffect, useCallback } from 'react';

export interface ShellOption {
  name: string;
  path: string;
  label: string;
}

export interface UIConfig {
  animations: boolean;
  theme: string;
  fontFamily: string;
  fontSize: number;
  borderRadius: number;
  navBarHeight: number;
  paneHeaderHeight: number;
}

export interface TerminalConfig {
  shell: string;
  shells: ShellOption[];
  fontFamily: string;
  fontSize: number;
  scrollback: number;
  cursorBlink: boolean;
  cursorStyle: string;
}

export interface PanesConfig {
  defaultWidth: number;
  gap: number;
  peek: number;
  insertPosition: string;
  default: Array<{ id: string; type: string; title: string; width: number; order: number }>;
}

export interface BrowserConfig {
  homepage: string;
  bookmarks: Array<{ name: string; url: string }>;
}

export interface KeybindingsConfig {
  mode: 'default' | 'vim';
  leader: string;
}

export interface Config {
  ui: UIConfig;
  terminal: TerminalConfig;
  panes: PanesConfig;
  browser: BrowserConfig;
  keybindings: KeybindingsConfig;
}

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
    shells: [],
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
  browser: {
    homepage: 'https://google.com',
    bookmarks: [],
  },
  keybindings: {
    mode: 'default',
    leader: 'ctrl',
  },
};

let cachedConfig: Config | null = null;

export function useConfig() {
  const [config, setConfig] = useState<Config>(cachedConfig ?? DEFAULT_CONFIG);
  const [loaded, setLoaded] = useState(cachedConfig !== null);

  useEffect(() => {
    if (cachedConfig) return;
    window.electronAPI.getConfig()
      .then((cfg) => {
        cachedConfig = cfg as Config;
        setConfig(cfg as Config);
        setLoaded(true);
      })
      .catch(() => {
        setLoaded(true);
      });
  }, []);

  // Listen for config updates from other components
  useEffect(() => {
    const handler = (e: Event) => {
      const cfg = (e as CustomEvent).detail as Config;
      cachedConfig = cfg;
      setConfig(cfg);
    };
    window.addEventListener('config-updated', handler);
    return () => window.removeEventListener('config-updated', handler);
  }, []);

  const reload = useCallback(() => {
    window.electronAPI.reloadConfig()
      .then((cfg) => {
        cachedConfig = cfg as Config;
        setConfig(cfg as Config);
        window.dispatchEvent(new CustomEvent('config-updated', { detail: cfg }));
      })
      .catch(console.error);
  }, []);

  const save = useCallback((partial: Partial<Config>) => {
    return window.electronAPI.saveConfig(partial)
      .then((cfg) => {
        cachedConfig = cfg as Config;
        setConfig(cfg as Config);
        window.dispatchEvent(new CustomEvent('config-updated', { detail: cfg }));
        return cfg as Config;
      });
  }, []);

  return { config, loaded, reload, save };
}
