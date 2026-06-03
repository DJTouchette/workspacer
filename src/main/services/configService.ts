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

interface AppEntry {
  name: string;
  url: string;
  icon?: string;
}

interface ScriptEntry {
  name: string;
  command: string;
}

interface Config {
  ui: {
    animations: boolean;
    theme: string;
    /** User override for corner style ('' = use the theme's own default). */
    cornerStyle: string;
    /** User override for the focused-pane border color ('' = theme default). */
    borderColor: string;
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
    hibernateAfter: number;
  };
  panes: {
    defaultWidth: number;
    gap: number;
    peek: number;
    insertPosition: string;
    tabPosition: string; // 'top' | 'left'
    default: Array<{ id: string; type: string; title: string; width: number; order: number }>;
  };
  keybindings: {
    mode: string;
    leader: string;
    shortcuts: Record<string, string>;
  };
  notifications: {
    /** Master switch for OS notifications + taskbar attention. */
    enabled: boolean;
    /** Also notify when an agent finishes (working → idle), not just needs-you. */
    notifyDone: boolean;
    /** Suppress notifications for the agent currently on screen. */
    onlyWhenUnwatched: boolean;
    /** Play the OS notification sound. */
    sound: boolean;
  };
  claude: {
    /** Default `--model` for new agents ('' = Claude Code's own default). */
    defaultModel: string;
    /** Concrete model ids observed in transcripts, to enrich the spawn dropdown. */
    seenModels: string[];
    /** Default for the spawn dialog's `--dangerously-skip-permissions` toggle. */
    skipPermissionsDefault: boolean;
  };
  /** Directories surfaced in the Overview pane for quick agent launching. */
  directories: {
    recent: string[];
    favourites: string[];
  };
  /** Per-directory script buttons, keyed by workspace root (normalized cwd). */
  scripts: Record<string, ScriptEntry[]>;
  apps: AppEntry[];
  session: {
    /** Restore the most recent session automatically on launch (skip the picker). */
    autoResume: boolean;
  };
}

function defaultShells(): ShellOption[] {
  if (process.platform === 'win32') {
    return [
      { name: 'gitbash', path: 'C:\\Program Files\\Git\\bin\\bash.exe', label: 'Git Bash' },
      { name: 'powershell', path: 'powershell.exe', label: 'PowerShell' },
      { name: 'pwsh', path: 'pwsh.exe', label: 'PowerShell 7' },
      { name: 'cmd', path: 'cmd.exe', label: 'Command Prompt' },
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
      cornerStyle: '',
      borderColor: '',
      fontFamily: 'Inter, system-ui, sans-serif',
      fontSize: 14,
      borderRadius: 8,
      navBarHeight: 34,
      paneHeaderHeight: 22,
    },
    terminal: {
      shell: '',
      shells: defaultShells(),
      fontFamily: '"JetBrainsMono Nerd Font Mono", "JetBrainsMonoNL Nerd Font Mono", "JetBrainsMono NFM", "JetBrainsMonoNL NFM", "JetBrainsMono NF", "CaskaydiaMono Nerd Font Mono", "CaskaydiaCove Nerd Font Mono", "CaskaydiaMono NF", "Cascadia Mono", monospace',
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
      hibernateAfter: 300,
    },
    panes: {
      defaultWidth: 800,
      gap: 16,
      peek: 80,
      insertPosition: 'after',
      tabPosition: 'top',
      default: [
        { id: 'terminal-1', type: 'terminal', title: 'Terminal 1', width: 800, order: 0 },
        { id: 'terminal-2', type: 'terminal', title: 'Terminal 2', width: 800, order: 1 },
        { id: 'terminal-3', type: 'terminal', title: 'Terminal 3', width: 800, order: 2 },
        { id: 'notes-1', type: 'notes', title: 'Notes', width: 800, order: 3 },
      ],
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
        'library-picker': 'ctrl+shift+l',
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
        'prev-agent': 'ctrl+alt+arrowup',
        'next-agent': 'ctrl+alt+arrowdown',
        'next-attention': 'ctrl+alt+arrowright',
        'spawn-agent': 'ctrl+alt+n',
      },
    },
    notifications: {
      enabled: true,
      notifyDone: true,
      onlyWhenUnwatched: true,
      sound: false,
    },
    claude: {
      defaultModel: '',
      seenModels: [],
      skipPermissionsDefault: false,
    },
    directories: {
      recent: [],
      favourites: [],
    },
    scripts: {},
    session: {
      autoResume: true,
    },
    apps: [
      { name: 'GitHub', url: 'https://github.com', icon: '\u{1F4BB}' },
      { name: 'ChatGPT', url: 'https://chat.openai.com', icon: '\u{1F916}' },
      { name: 'Claude', url: 'https://claude.ai', icon: '\u{2728}' },
      { name: 'Stack Overflow', url: 'https://stackoverflow.com', icon: '\u{1F4DA}' },
      { name: 'Localhost 3000', url: 'http://localhost:3000', icon: '\u{1F310}' },
      { name: 'Localhost 8080', url: 'http://localhost:8080', icon: '\u{1F310}' },
    ],
  };
}

export function getConfigDir(): string {
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

  saveConfig(partial: Partial<Config>): Config {
    this.config = deepMerge(this.config, partial);
    try {
      const dir = getConfigDir();
      fs.mkdirSync(dir, { recursive: true });
      const data = yaml.dump(this.config, { lineWidth: -1 });
      fs.writeFileSync(getConfigFilePath(), data, 'utf-8');
    } catch (err) {
      console.error('[ConfigService] failed to save config:', err);
    }
    return this.config;
  }

  getConfigPath(): string {
    return getConfigFilePath();
  }
}

export const configService = new ConfigService();
