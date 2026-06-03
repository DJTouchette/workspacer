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
  tabPosition: 'top' | 'left';
  default: Array<{ id: string; type: string; title: string; width: number; order: number }>;
}

export interface BrowserConfig {
  homepage: string;
  bookmarks: Array<{ name: string; url: string }>;
  hibernateAfter: number;
}

export interface AppEntry {
  name: string;
  url: string;
  icon?: string;
}

export interface KeybindingsConfig {
  mode: 'default' | 'vim';
  leader: string;
  shortcuts: Record<string, string>;
}

export interface NotificationsConfig {
  enabled: boolean;
  notifyDone: boolean;
  onlyWhenUnwatched: boolean;
  sound: boolean;
}

export interface ScriptEntry {
  name: string;
  command: string;
}

export interface Config {
  ui: UIConfig;
  terminal: TerminalConfig;
  panes: PanesConfig;
  browser: BrowserConfig;
  keybindings: KeybindingsConfig;
  notifications: NotificationsConfig;
  /** Per-directory script buttons, keyed by workspace root (normalized cwd). */
  scripts: Record<string, ScriptEntry[]>;
  apps: AppEntry[];
  /** Directories surfaced in the Overview pane for quick agent launching. */
  directories?: {
    recent: string[];
    favourites: string[];
  };
  session?: {
    /** Restore the most recent session automatically on launch (skip the picker). */
    autoResume: boolean;
  };
}

const DEFAULT_CONFIG: Config = {
  ui: {
    animations: false,
    theme: 'dark',
    fontFamily: 'Inter, system-ui, sans-serif',
    fontSize: 14,
    borderRadius: 8,
    navBarHeight: 34,
    paneHeaderHeight: 22,
  },
  terminal: {
    shell: '',
    shells: [],
    fontFamily: '"JetBrainsMono Nerd Font Mono", "JetBrainsMono NF", "CaskaydiaMono Nerd Font Mono", "CaskaydiaMono NF", monospace',
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
    shortcuts: {
      'new-terminal': 'ctrl+t',
      'new-browser': 'ctrl+n',
      'new-claude': 'ctrl+j',
      'split': 'ctrl+d',
      'quick-split': 'ctrl+shift+d',
      'close-pane': 'ctrl+w',
      'command-palette': 'ctrl+k',
      'settings': 'ctrl+,',
      'save-session': 'ctrl+s',
      'rename-tab': 'f2',
      'toggle-help': 'ctrl+?',
      'prev-tab': 'ctrl+[',
      'next-tab': 'ctrl+]',
      'nav-left': 'ctrl+h',
      'nav-right': 'ctrl+l',
      'nav-up': 'ctrl+shift+k',
      'nav-down': 'ctrl+shift+j',
    },
  },
  notifications: {
    enabled: true,
    notifyDone: true,
    onlyWhenUnwatched: true,
    sound: false,
  },
  scripts: {},
  apps: [],
  session: { autoResume: true },
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
