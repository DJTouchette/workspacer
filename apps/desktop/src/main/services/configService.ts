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
    viewMode: string; // 'tabs' | 'spatial' | 'stacked'
    viewLevel?: string; // 'piloting' | 'fleet'
    default: Array<{ id: string; type: string; title: string; width: number; order: number }>;
  };
  keybindings: {
    /** Workspace prefix combo (default 'ctrl+space'). */
    prefix: string;
    /** Expand the chord indicator into a which-key cheatsheet. Default true. */
    chordHints?: boolean;
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
    /** Which view a Claude pane opens in by default: rich 'gui' or raw 'terminal'. */
    defaultView: 'gui' | 'terminal';
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
  editor: {
    /** Editor-pane engine: in-app 'codemirror', or your $EDITOR in a 'terminal'. */
    engine: 'codemirror' | 'terminal';
    /** Command for the 'terminal' engine; the file path is appended as its last arg. */
    terminalCommand: string;
    /** Vim keybindings inside the CodeMirror editor. */
    vim?: boolean;
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

/**
 * Default keybindings, prefix-forward. Values are either direct combos
 * (terminal-safe keys only) or prefix chords ('prefix <key>' — press the
 * workspace prefix, then the key). Kept in sync with the renderer's
 * DEFAULT_SHORTCUTS (hooks/useConfig.ts).
 */
const DEFAULT_SHORTCUTS: Record<string, string> = {
  // Direct, terminal-safe
  'command-palette': 'ctrl+shift+p',
  'next-agent': 'ctrl+tab',
  'prev-agent': 'ctrl+shift+tab',
  'next-attention': 'ctrl+shift+space',
  'spawn-agent': 'ctrl+shift+n',
  'settings': 'ctrl+,',
  'save-session': 'ctrl+shift+s',
  'open-file': 'ctrl+shift+o',
  'toggle-help': 'f1',
  'toggle-terminal': 'ctrl+`',
  'toggle-sidebar': 'ctrl+shift+b',
  'toggle-inbox': 'ctrl+shift+i',
  'toggle-fleet': 'ctrl+shift+f',
  'toggle-inspector': 'ctrl+shift+e',
  'library-picker': 'ctrl+shift+l',
  'open-review': 'ctrl+shift+g',
  // Prefix chords (Ctrl+Space then …), grouped into submenus
  // New ▸
  'new-terminal': 'prefix n t',
  'new-claude': 'prefix n c',
  'new-browser': 'prefix n b',
  // Tab ▸
  'prev-tab': 'prefix t [',
  'next-tab': 'prefix t ]',
  'move-tab-left': 'prefix t ,',
  'move-tab-right': 'prefix t .',
  'rename-tab': 'prefix t r',
  'close-pane': 'prefix t w',
  // Pane ▸
  'split': 'prefix p s',
  'quick-split': 'prefix p c',
  'nav-left': 'prefix p h',
  'nav-down': 'prefix p j',
  'nav-up': 'prefix p k',
  'nav-right': 'prefix p l',
  // Top-level leaf
  'cycle-view': 'prefix v',
};

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
      scrollback: 1500,
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
      viewMode: 'tabs',
      viewLevel: 'piloting',
      default: [
        { id: 'terminal-1', type: 'terminal', title: 'Terminal 1', width: 800, order: 0 },
        { id: 'terminal-2', type: 'terminal', title: 'Terminal 2', width: 800, order: 1 },
        { id: 'terminal-3', type: 'terminal', title: 'Terminal 3', width: 800, order: 2 },
        { id: 'notes-1', type: 'notes', title: 'Notes', width: 800, order: 3 },
      ],
    },
    keybindings: {
      prefix: 'ctrl+space',
      chordHints: true,
      shortcuts: { ...DEFAULT_SHORTCUTS },
    },
    notifications: {
      enabled: true,
      notifyDone: true,
      onlyWhenUnwatched: true,
      sound: false,
    },
    editor: {
      engine: 'codemirror',
      terminalCommand: 'nvim',
    },
    claude: {
      defaultModel: '',
      seenModels: [],
      skipPermissionsDefault: false,
      defaultView: 'terminal',
    },
    directories: {
      recent: [],
      favourites: [],
    },
    scripts: {},
    session: {
      autoResume: false,
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

/**
 * One-time migration from the old keybindings schema (mode/leader + Ctrl-letter
 * map) to the prefix-forward scheme. The old defaults were written to disk on
 * first run, so without this every existing user would keep the legacy bindings
 * (and their terminal-stealing Ctrl+L/D/S). Resets keybindings wholesale and,
 * if the user had Vim keybinding mode on, preserves it as editor Vim.
 */
function migrateKeybindings(cfg: Config): Config {
  const kb = cfg.keybindings as { mode?: string; leader?: string; prefix?: string } | undefined;
  const isLegacy = !!kb && (kb.mode !== undefined || kb.leader !== undefined || !kb.prefix);
  if (!isLegacy) return cfg;

  const hadVim = kb?.mode === 'vim';
  cfg.keybindings = { prefix: 'ctrl+space', chordHints: true, shortcuts: { ...DEFAULT_SHORTCUTS } };
  if (hadVim) cfg.editor = { ...cfg.editor, vim: true };

  try {
    fs.mkdirSync(getConfigDir(), { recursive: true });
    fs.writeFileSync(getConfigFilePath(), yaml.dump(cfg, { lineWidth: -1 }), 'utf-8');
  } catch (err) {
    console.error('[ConfigService] keybindings migration write failed:', err);
  }
  return cfg;
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
      const merged = deepMerge(defaults, parsed) as Config;
      return migrateKeybindings(merged);
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
