import { useConfigContext } from '../contexts/ConfigContext';
import type { AgentProvider } from '../types/pane';

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
  /** Show the composer's send (↑) button. When off, Enter still sends — useful
   *  if the button gets in the way of input. Optional; absent = on. */
  showComposerSend?: boolean;
  /** Font scale for the GUI conversation view (1 = original size). Optional;
   *  absent = the default scale. */
  guiFontScale?: number;
  /** How GUI diffs are laid out: 'stacked' (removed block then added block),
   *  'inline' (interleaved unified), or 'split' (side-by-side). Absent = stacked. */
  diffView?: 'stacked' | 'inline' | 'split';
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
  /** Defaults applied when spawning a new agent. */
  agents?: {
    /** Coding-agent backend pre-selected in the spawn dialog. */
    defaultProvider?: AgentProvider;
    /** Directory the spawn dialog opens at (and where Browse… starts). Absent
     *  falls back to the app's launch cwd. Set this so new agents don't default
     *  to the install path. */
    defaultCwd?: string;
  };
  /** Optional fleet-supervisor settings (opt-in; absent = sensible defaults). */
  supervisor?: SupervisorConfig;
}

export interface SupervisorConfig {
  /** Coordinator model for supervisor sessions ('' = the app/Claude default). */
  model?: string;
  /** Cheap model the supervisor spawns for transcript digests (e.g. 'sonnet'). */
  summarizerModel?: string;
  /** How often (seconds) the supervisor's loop re-sweeps the fleet. */
  pollSeconds?: number;
  /** Coding-agent backend the supervisor runs on. undefined ⇒ 'claude'.
   *  Non-Claude supervisors run the chosen CLI but the workspacer MCP facade
   *  (the supervisor's fleet-coordination tools) is currently Claude-only. */
  provider?: AgentProvider;
}

/**
 * Default keybindings + config live in ./configDefaults (a dependency leaf) so
 * this module stays out of an import cycle with ConfigContext. Re-exported here
 * so existing `import { DEFAULT_CONFIG } from '../hooks/useConfig'` callers keep
 * working unchanged.
 */
export { DEFAULT_SHORTCUTS, DEFAULT_CONFIG } from './configDefaults';

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
