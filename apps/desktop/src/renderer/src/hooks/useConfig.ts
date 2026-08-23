import { useConfigContext } from '../contexts/ConfigContext';
import type { AgentProvider } from '../types/pane';
import type { CustomThemes } from '../themes';
import type { WidgetPlacement } from '../types/widget';

export interface ShellOption {
  name: string;
  path: string;
  label: string;
}

export interface UIConfig {
  animations: boolean;
  theme: string;
  /** User-made themes, keyed by namespaced id ('custom:<slug>'). Each stores a
   *  display name, the built-in it was forked from, and a fully resolved flat
   *  token map — see resolveTheme() in themes.ts. */
  customThemes?: CustomThemes;
  /** User override for corner style ('' = use the theme's own default). */
  cornerStyle: string;
  /** User override for the focused-pane border color ('' = theme default). */
  borderColor: string;
  /** UI typeface: a bundled id ('hanken' | 'inter' | …, see lib/uiFont) or
   *  'custom:<Family>' for an uploaded font. Unknown values → default. */
  fontFamily: string;
  fontSize: number;
  borderRadius: number;
  navBarHeight: number;
  /** Expanded sidebar width in px. Set by dragging the sidebar's right edge;
   *  clamped on read (see lib/sidebarWidth). Absent = the shipped default. */
  sidebarWidth?: number;
  paneHeaderHeight: number;
  /** Show the composer's send (↑) button. When off, Enter still sends — useful
   *  if the button gets in the way of input. Optional; absent = on. */
  showComposerSend?: boolean;
  /** Font scale for the GUI conversation view (1 = original size). Optional;
   *  absent = the default scale. */
  guiFontScale?: number;
  /** App-wide text scale (1 = default). Applied as the document root
   *  font-size, so every rem-sized text in the app follows. Adjustable from
   *  Settings → Appearance and mod+= / mod+- / mod+0. */
  uiFontScale?: number;
  /** Harpoon-style agent pins (command layer): SESSION-id slots, array order
   *  = slot number (prefix 1-9 jumps, prefix m toggles, ⚓N badges in the
   *  sidebar; the TUI's harpoon reads the same key). Session-keyed because a
   *  cwd is ambiguous the moment two agents share a directory — and session
   *  ids are stable here: respawns resume the same pinned id, so a pin
   *  survives stop/respawn and dies with an explicit close. An ARRAY on
   *  purpose — deepMerge and configPatch replace arrays wholesale, so
   *  unpinning round-trips without wholesale-path machinery. */
  pinnedAgentSessions?: string[];
  /** DEPRECATED (lived for a few hours of nightlies): the cwd-keyed
   *  predecessor of pinnedAgentSessions. Read once for migration (resolved
   *  against live agents, then emptied); never written again. */
  pinnedAgentCwds?: string[];
  /** One-time in-app announcement flag for the command layer (Phase 5). */
  commandLayerAnnounced?: boolean;
  /** How GUI diffs are laid out: 'stacked' (removed block then added block),
   *  'inline' (interleaved unified), or 'split' (side-by-side). Absent = stacked. */
  diffView?: 'stacked' | 'inline' | 'split';
  /** App-wide UI mode: 'fleet' (full mission-control chrome) or 'focus'
   *  (minimal — rail sidebar, no inspector rail / Fleet Deck). Absent = fleet.
   *  See lib/uiMode.ts for the per-mode manifest. */
  mode?: 'fleet' | 'focus';
  /** What the pane-creation menus (in-pane "Split into…" + the "+" new-tab
   *  dropdown) list. Each id is a built-in pane type ('claude' | 'terminal' |
   *  'browser' | 'review' | 'library') or a plugin pane's `type`. Absent = the
   *  built-in default set plus every contributed plugin pane; an explicit array
   *  (even empty) is used verbatim, in order. See lib/paneMenu.ts. */
  paneMenu?: string[];
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
  /** The keybinding preset last applied ('vscode' | 'vim' | 'jetbrains'). Used
   *  by the Settings picker to show the active preset and to preserve user
   *  rebinds when switching presets. Absent on legacy configs. */
  presetId?: string;
  /** The opt-in tmux-style command layer (COMMAND_LAYER.md). */
  commandLayer?: CommandLayerConfig;
}

/** Settings for the transient command layer — a tmux-style armed key layer on
 *  the chord leader. All timings configurable (motor accessibility). */
export interface CommandLayerConfig {
  /** THE switch. Off = the dispatcher behaves exactly as before. */
  enabled?: boolean;
  /** Chord idle timeout while enabled; 0 = armed until resolved (Esc/mouse/
   *  unknown key disarm). Disabled layer keeps the legacy 1500ms. */
  timeoutMs?: number;
  /** After a repeat-group action (pane nav, tab cycling) the layer re-arms for
   *  this long so `prefix h h l` walks panes without re-pressing the leader. */
  repeatMs?: number;
  /** Dwell before the armed strip expands into the full HUD grid. */
  hudDelayMs?: number;
  /** Leader pressed again while armed sends the literal prefix byte to the
   *  terminal it was armed from (nested tmux). */
  passthrough?: boolean;
  /** Armed-state chrome: 'strip' (full command strip) | 'minimal' (bare chip).
   *  Never 'none' — an enabled layer must not have invisible armed state. */
  indicator?: 'strip' | 'minimal';
  /** Per-platform leader escape hatch (e.g. a Hyprland setup where the Alt tap
   *  collides with WM bindings); empty = the resolved default. */
  leaderOverride?: string;
}

export interface NotificationsConfig {
  enabled: boolean;
  notifyDone: boolean;
  onlyWhenUnwatched: boolean;
  sound: boolean;
  /** Show transient in-app toast popups for new notifications. */
  inAppToasts?: boolean;
}

export interface EditorConfig {
  /** How files open: 'codemirror' (legacy value — now the sandboxed editor
   *  plugin, falling back to the OS editor), or your $EDITOR in a 'terminal'. */
  engine: 'codemirror' | 'terminal';
  /** Command for the 'terminal' engine; the file path is appended as its last arg. */
  terminalCommand: string;
}

export interface ScriptEntry {
  name: string;
  command: string;
}

/**
 * How one project should read at a glance. `favicon` wins when it loads, `icon`
 * (an emoji or one or two letters) is next, and initials derived from the name
 * are the floor — so there is always something to draw. Mirrors ProjectIdentity
 * in main/services/configService.ts.
 */
export interface ProjectIdentity {
  label?: string;
  color?: string;
  icon?: string;
  favicon?: string;
  /**
   * The DOWNLOADED icon's filename under `<configDir>/project-icons/`, served
   * to the renderer as `workspacer-icon://<iconFile>`. This is what actually
   * renders; `favicon` is kept as the source it came from, so the field can
   * show what you pasted and the icon can be re-fetched.
   */
  iconFile?: string;
  /** Pinned by the user. Absent falls back to the legacy `directories.favourites`. */
  favourite?: boolean;
  /**
   * How the Fleet Manager lands work in this project (the standing per-project
   * delivery policy it reads at dispatch and bakes into each worker's brief):
   *   'pr'    — open a pull request for review (default, safest).
   *   'local' — land changes on a branch / local merge after your approval; no PR.
   * Absent = 'pr'. Advisory to the manager, not a hard gate.
   */
  delivery?: 'pr' | 'local';
  /**
   * Per-project full-access: workers the manager dispatches INTO this project
   * run with permissions bypassed (no per-action approvals). The narrower,
   * per-repo form of agents.fleetFullAccess. Absent = off. When any project
   * sets this, the manager's token gets the hub-verified yolo grant; the
   * manager still only bypasses for the flagged project (doctrine-enforced).
   */
  yolo?: boolean;
  /**
   * Shell commands run (in order) in a fresh agent worktree of this project,
   * right after `git worktree add` + the automatic node_modules linking. Each
   * runs with cwd = the worktree root, `$SOURCE` (the source checkout) and
   * `$WORKTREE` (the new worktree) substituted and exported, under a 5-minute
   * per-command timeout. `script:<name>` references this project's `scripts`
   * entry by name. The first failure stops the rest; a failed setup is a
   * warning, never a refused spawn.
   */
  worktreeSetup?: string[];
  /** Epoch ms this project was last opened. Absent falls back to the legacy
   *  `directories.recent` ordering. */
  lastOpened?: number;
  /**
   * Per-project settings belonging to plugins, namespaced by plugin id. Only
   * NON-SECRET values live here: config.yaml is credential-free by design, and
   * `config.get` is an unguarded bus capability on exactly that basis (see its
   * entry in capspec.go). A plugin's tokens stay in its own `.settings.json`.
   */
  plugins?: Record<string, Record<string, unknown>>;
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
  /** Per-directory widget boards, keyed the same way as `scripts` (see
   *  lib/projectKey). Absent for a project with no board yet. */
  widgets?: Record<string, WidgetPlacement[]>;
  /** Per-directory project identity (label / colour / icon / favicon), keyed by
   *  normalized cwd exactly as `scripts` and `widgets` are — see lib/projectKey.
   *  Absent for a project nobody has customized; the mark is derived instead. */
  projects?: Record<string, ProjectIdentity>;
  apps: AppEntry[];
  /** Directories surfaced in the Overview pane for quick agent launching. */
  directories?: {
    recent: string[];
    favourites: string[];
  };
  /** In-app auto-update (electron-updater). Only acts in packaged builds. */
  updates?: {
    /** Master switch for auto-update. Default true. */
    enabled: boolean;
    /** Release channel ('latest', 'beta', …). */
    channel?: string;
  };
  /** Set once the user dismisses the first-run welcome; absent/false shows it. */
  onboardingDismissed?: boolean;
  claude?: {
    /** Which view a Claude pane opens in by default: rich 'gui' or raw 'terminal'. */
    defaultView: 'gui' | 'terminal';
    /** How runs of tool calls render in the GUI: prose summary cards, or the
     *  waterfall trace monitor (per-call duration bars + dig-in rows). */
    workLog?: 'cards' | 'trace';
    /** Show a small HH:MM stamp on chat turns in the GUI conversation. */
    showTimestamps?: boolean;
    /** Render the file contents a Read tool call returned, inline in the
     *  expanded work log. Off = just the "Read <file>" line. Default true. */
    showFileReads?: boolean;
    /** Concrete model ids seen across sessions, surfaced in the spawn dropdown. */
    seenModels?: string[];
    /** Permission mode pre-selected in the spawn dialog ('' = provider
     *  default, i.e. "Ask to approve"). Set from Settings → Session; also what
     *  the hub facade resolves an OMITTED spawn_agent skipPermissions from. */
    defaultPermissionMode?: string;
    /** The bypass spelling of defaultPermissionMode: true ⇒ new agents start
     *  with `--dangerously-skip-permissions`. Settings writes the two together
     *  (see SessionSection) so they can never contradict. */
    skipPermissionsDefault?: boolean;
    /** How new Claude sessions run: 'pty' (classic Claude Code TUI — Term +
     *  GUI) or 'stream' (headless stream-json via claudemon's managed adapter —
     *  GUI only). Per-spawn overridable in the spawn dialog. Default 'stream'. */
    transport?: 'pty' | 'stream';
    /** Experimental: install claudemon's hooks + statusLine into a private
     *  overlay file passed to Claude via --settings, instead of mutating the
     *  user's global ~/.claude/settings.json. Default off. */
    settingsOverlay?: boolean;
    /** Optional per-session cost budgets (USD), keyed by session id. Set from
     *  the inspector; an OS notification fires once when spend crosses it. */
    budgets?: Record<string, number>;
    /** Keep a subscription 5-hour rate-limit window warm: when the current
     *  window is expired/absent (0%), fire one minimal Haiku ping so a fresh
     *  window is already running by the time real work starts. Off by default;
     *  runs only while Workspacer is open. */
    keepWarm?: {
      enabled: boolean;
      /** Which subscription windows to warm ('claude' and/or 'codex'). Both
       *  meter usage in 5h windows that start with the first message. */
      providers?: string[];
      /** 'auto' = re-warm whenever the window lapses; 'interval' = check every
       *  `intervalHours`; 'daily' = check once a day at `dailyAt`. Every mode
       *  checks account usage first and skips while a window is active. */
      mode: 'auto' | 'interval' | 'daily';
      /** Hours between checks in 'interval' mode. */
      intervalHours: number;
      /** Local "HH:MM" for the once-a-day check in 'daily' mode. */
      dailyAt: string;
    };
  };
  /** Defaults applied when spawning a new agent. */
  agents?: {
    /** Coding-agent backend pre-selected in the spawn dialog. */
    defaultProvider?: AgentProvider;
    /** Directory the spawn dialog opens at (and where Browse… starts). Absent
     *  falls back to the app's launch cwd. Set this so new agents don't default
     *  to the install path. */
    defaultCwd?: string;
    /** The Fleet Manager's home: the parent directory holding your projects.
     *  '' = derive from config.projects (their common parent), else $HOME. */
    fleetRoot?: string;
    /** Full-access dispatch: when true, the Fleet Manager and the workers it
     *  dispatches run with permissions bypassed (no per-action approvals) — the
     *  manager's session token carries a hub-verified yolo grant. Default off:
     *  workers prompt, and the manager approves in-repo actions per its
     *  doctrine. Trade-off is speed vs. a human gate on every command. */
    fleetFullAccess?: boolean;
    /** The harness the Fleet Manager ITSELF runs on (absent = claude). The
     *  manager dispatches through the workspacer MCP facade, so this is limited
     *  to providers with an MCP client. Applies to the next manager you start —
     *  a running/stopped manager keeps its own harness (its conversation cannot
     *  move between them). */
    managerProvider?: AgentProvider;
    /** Pre-check "isolated worktree" in the spawn dialog. */
    spawnInWorktree?: boolean;
    /** Parent directory for agent worktrees ('' = ~/.workspacer/worktrees). */
    worktreeRoot?: string;
    /** User-configured binary paths per provider. '' = auto-detect on PATH. */
    binaries?: {
      claude?: string;
      codex?: string;
      opencode?: string;
      pi?: string;
    };
    /** Name new agents after their first exchange, like a chat service names a
     *  conversation. A rename you type always wins and is never overwritten. */
    autoTitle?: {
      /** Absent/true = on. */
      enabled?: boolean;
      /** Model for the one-shot title call (a cheap one; '' = claude default). */
      model?: string;
    };
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
  /** Full access: the supervisor runs with permissions bypassed and the
   *  summarizer workers it spawns inherit the bypass (its facade token carries
   *  the yolo grant). The supervisor twin of agents.fleetFullAccess. */
  fullAccess?: boolean;
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
