import { useConfigContext } from '../contexts/ConfigContext';

export interface ShellOption {
  name: string;
  path: string;
  label: string;
}

export interface UIConfig {
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
  /** Global layout paradigm: 'tabs' strip, 'spatial' canvas, or 'stacked' feed. */
  viewMode: 'tabs' | 'spatial' | 'stacked';
  /** Global altitude: 'piloting' one agent, or the cross-agent 'fleet' deck. */
  viewLevel?: 'fleet' | 'piloting';
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
  /** Per-action combo overrides. Optional: recomputed from mode/leader at runtime
   *  and omitted from partial saves; all readers use optional chaining. */
  shortcuts?: Record<string, string>;
}

export interface NotificationsConfig {
  enabled: boolean;
  notifyDone: boolean;
  onlyWhenUnwatched: boolean;
  sound: boolean;
}

export interface EditorConfig {
  /** Editor-pane engine: in-app 'codemirror', or your $EDITOR in a 'terminal'. */
  engine: 'codemirror' | 'terminal';
  /** Command for the 'terminal' engine; the file path is appended as its last arg. */
  terminalCommand: string;
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
  editor?: EditorConfig;
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
  /** Set once the user dismisses the first-run welcome; absent/false shows it. */
  onboardingDismissed?: boolean;
  claude?: {
    /** Which view a Claude pane opens in by default: rich 'gui' or raw 'terminal'. */
    defaultView: 'gui' | 'terminal';
    /** Concrete model ids seen across sessions, surfaced in the spawn dropdown. */
    seenModels?: string[];
  };
}

export const DEFAULT_CONFIG: Config = {
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
    viewMode: 'tabs',
    viewLevel: 'piloting',
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
      'library-picker': 'ctrl+l',
      'toggle-terminal': 'ctrl+`',
      'toggle-sidebar': 'ctrl+b',
      'toggle-inbox': 'ctrl+shift+a',
      'toggle-fleet': 'ctrl+shift+f',
      'toggle-inspector': 'ctrl+shift+e',
      'settings': 'ctrl+,',
      'save-session': 'ctrl+s',
      'rename-tab': 'f2',
      'toggle-help': 'ctrl+?',
      'prev-tab': 'ctrl+[',
      'next-tab': 'ctrl+]',
      'nav-left': 'ctrl+h',
      'nav-right': 'ctrl+shift+l',
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
  session: { autoResume: false },
  claude: { defaultView: 'terminal' },
};

/**
 * Access the application config.
 *
 * Must be rendered inside <ConfigProvider>.  Returns { config, loaded, reload, save }.
 * The public API is identical to the previous module-singleton implementation;
 * all consumers continue to work without changes.
 */
export function useConfig() {
  return useConfigContext();
}
