import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import * as yaml from 'js-yaml';

interface ShellOption {
  name: string;
  path: string;
  label: string;
}

interface Bookmark {
  name: string;
  url: string;
}

interface Config {
  ui: {
    animations: boolean;
    theme: string;
    fontFamily: string;
    fontSize: number;
    borderRadius: number;
    navBarHeight: number;
    paneHeaderHeight: number;
  };
  terminal: {
    shell: string;
    shells: ShellOption[];
    fontFamily: string;
    fontSize: number;
    scrollback: number;
    cursorBlink: boolean;
    cursorStyle: string;
  };
  browser: {
    homepage: string;
    bookmarks: Bookmark[];
  };
  panes: {
    defaultWidth: number;
    gap: number;
    peek: number;
    insertPosition: string;
    default: Array<{ id: string; type: string; title: string; width: number; order: number }>;
  };
}

function defaultShells(): ShellOption[] {
  if (process.platform === 'win32') {
    return [
      { name: 'powershell', path: 'powershell.exe', label: 'PowerShell' },
      { name: 'pwsh', path: 'pwsh.exe', label: 'PowerShell 7' },
      { name: 'cmd', path: 'cmd.exe', label: 'Command Prompt' },
      { name: 'gitbash', path: 'C:\\Program Files\\Git\\bin\\bash.exe', label: 'Git Bash' },
      { name: 'wsl', path: 'wsl.exe', label: 'WSL' },
    ];
  }
  return [
    { name: 'default', path: '', label: 'Default ($SHELL)' },
    { name: 'bash', path: '/bin/bash', label: 'Bash' },
    { name: 'zsh', path: '/bin/zsh', label: 'Zsh' },
    { name: 'fish', path: '/usr/bin/fish', label: 'Fish' },
  ];
}

function defaultConfig(): Config {
  return {
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
      shells: defaultShells(),
      fontFamily: 'JetBrainsMono NF, JetBrainsMono Nerd Font, CaskaydiaMono NF, monospace',
      fontSize: 14,
      scrollback: 5000,
      cursorBlink: true,
      cursorStyle: 'block',
    },
    browser: {
      homepage: 'https://google.com',
      bookmarks: [
        { name: 'Go Docs', url: 'https://pkg.go.dev' },
        { name: 'MDN', url: 'https://developer.mozilla.org' },
        { name: 'Localhost 3000', url: 'http://localhost:3000' },
        { name: 'Localhost 8080', url: 'http://localhost:8080' },
      ],
    },
    panes: {
      defaultWidth: 800,
      gap: 16,
      peek: 80,
      insertPosition: 'after',
      default: [
        { id: 'terminal-1', type: 'terminal', title: 'Terminal 1', width: 800, order: 0 },
        { id: 'terminal-2', type: 'terminal', title: 'Terminal 2', width: 800, order: 1 },
        { id: 'terminal-3', type: 'terminal', title: 'Terminal 3', width: 800, order: 2 },
        { id: 'notes-1', type: 'notes', title: 'Notes', width: 800, order: 3 },
      ],
    },
  };
}

function getConfigDir(): string {
  if (process.platform === 'win32') {
    const appData = process.env.APPDATA;
    if (appData) return path.join(appData, 'workspacer');
    return path.join(os.homedir(), 'AppData', 'Roaming', 'workspacer');
  }
  const xdg = process.env.XDG_CONFIG_HOME;
  if (xdg) return path.join(xdg, 'workspacer');
  return path.join(os.homedir(), '.config', 'workspacer');
}

function getConfigFilePath(): string {
  return path.join(getConfigDir(), 'config.yaml');
}

// Deep merge source into target, preserving target defaults for missing keys
function deepMerge(target: any, source: any): any {
  if (!source || typeof source !== 'object') return target;
  const result = { ...target };
  for (const key of Object.keys(source)) {
    if (
      source[key] &&
      typeof source[key] === 'object' &&
      !Array.isArray(source[key]) &&
      target[key] &&
      typeof target[key] === 'object' &&
      !Array.isArray(target[key])
    ) {
      result[key] = deepMerge(target[key], source[key]);
    } else {
      result[key] = source[key];
    }
  }
  return result;
}

class ConfigService {
  private config: Config;

  constructor() {
    this.config = this.loadFromDisk();
  }

  private loadFromDisk(): Config {
    const defaults = defaultConfig();
    const configPath = getConfigFilePath();

    try {
      const data = fs.readFileSync(configPath, 'utf-8');
      const parsed = yaml.load(data) as Partial<Config>;
      return deepMerge(defaults, parsed);
    } catch {
      // No config file — write defaults
      this.writeDefaults();
      return defaults;
    }
  }

  private writeDefaults(): void {
    try {
      const dir = getConfigDir();
      fs.mkdirSync(dir, { recursive: true });
      const data = yaml.dump(defaultConfig(), { lineWidth: -1 });
      fs.writeFileSync(getConfigFilePath(), data, 'utf-8');
    } catch (err) {
      console.error('[ConfigService] failed to write default config:', err);
    }
  }

  getConfig(): Config {
    return this.config;
  }

  reloadConfig(): Config {
    this.config = this.loadFromDisk();
    return this.config;
  }

  getConfigPath(): string {
    return getConfigFilePath();
  }
}

export const configService = new ConfigService();
