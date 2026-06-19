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
  /** Workspace prefix combo (default 'ctrl+space'). Any binding whose value
   *  starts with the literal token `prefix ` fires as a two-step chord: press
   *  the prefix, then the rest of the combo. Bindings without it are direct. */
  prefix: string;
  /** Per-action combo overrides, merged over defaults. A value is either a
   *  direct combo ('ctrl+shift+p') or a prefix chord ('prefix n'). */
  shortcuts?: Record<string, string>;
  /** Expand the chord indicator into a which-key cheatsheet of the available
   *  prefix chords. Default true. */
  chordHints?: boolean;
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
  /** Enable Vim keybindings inside the CodeMirror editor. Independent of the
   *  workspace keybindings (which no longer have a "vim mode"). */
  vim?: boolean;
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

/**
 * Default keybindings, prefix-forward. Two kinds of value:
 *  - direct combos ('ctrl+shift+p') — only terminal-safe keys live here, so a
 *    focused terminal/Claude TUI keeps Ctrl+C/D/L/S/W/etc. for itself.
 *  - prefix chords ('prefix n') — press the workspace prefix (Ctrl+Space),
 *    then the key. All structural tab/pane ops live behind the prefix.
 */
export const DEFAULT_SHORTCUTS: Record<string, string> = {
  // ── Direct, terminal-safe ──
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
  // ── Prefix chords (Ctrl+Space then …), grouped into submenus ──
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
    scrollback: 1500,
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
