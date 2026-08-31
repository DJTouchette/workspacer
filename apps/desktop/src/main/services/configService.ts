import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import * as yaml from 'js-yaml';
import { CONFIG_DEFAULTS } from './configDefaults.generated';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { withConfigLock } from '../lib/configLock';
import { pruneOrphanedConfigKeys } from '../lib/orphanedConfigKeys';
import { WHOLESALE_CONFIG_PATHS } from '../shared/configWholesale';
import type { ProjectIdentity } from '../shared/ipcTypes';
import { ModelSelectionError, normalizeModelSelection } from '../shared/modelContextWindows';

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

/**
 * One widget placed on a project's board. Mirrors the renderer's WidgetPlacement
 * (src/renderer/src/types/widget.ts) — keep in sync.
 *
 * `plugin` absent means a host (built-in) widget, which keeps the YAML readable:
 *   - { widget: git, size: large }
 *   - { plugin: djtouchette.shiplight, widget: lamp, size: small }
 */
interface WidgetPlacementEntry {
  plugin?: string;
  widget: string;
  size: 'small' | 'medium' | 'large';
}

// ProjectIdentity (per-project label/color/icon/yolo/delivery/worktreeSetup/…)
// is declared once in ../shared/ipcTypes.ts — the main↔renderer contract file
// — and imported here, so this side can't silently drift from the renderer's
// copy (useConfig.ts) the way it did before (see the `delivery` field, which
// existed only in the renderer's copy).

interface Config {
  ui: {
    animations: boolean;
    theme: string;
    /** User-made themes keyed by namespaced id ('custom:<slug>'). Renderer-
     *  owned shape (see renderer themes.ts); saved wholesale, never merged. */
    customThemes?: Record<string, { name: string; base?: string; colors: Record<string, unknown> }>;
    /** User override for corner style ('' = use the theme's own default). */
    cornerStyle: string;
    /** User override for the focused-pane border color ('' = theme default). */
    borderColor: string;
    /** UI typeface: a bundled id ('hanken' | 'inter' | …) or 'custom:<Family>'
     *  for an uploaded font. Unknown values render as the default. */
    fontFamily: string;
    fontSize: number;
    borderRadius: number;
    navBarHeight: number;
    paneHeaderHeight: number;
    showComposerSend?: boolean;
    /** Font scale for the GUI conversation view (1 = original size). */
    guiFontScale?: number;
    /** GUI diff layout: 'stacked' | 'inline' | 'split'. Absent = stacked. */
    diffView?: 'stacked' | 'inline' | 'split';
    /** App-wide UI mode: 'fleet' (full mission-control chrome) or 'focus'
     *  (minimal — rail sidebar, no inspector rail / Fleet Deck). */
    mode?: 'fleet' | 'focus';
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
    viewLevel?: string; // 'piloting' | 'fleet'
    default: Array<{ id: string; type: string; title: string; width: number; order: number }>;
  };
  keybindings: {
    /** Workspace prefix combo (default 'mod+space'). */
    prefix: string;
    /** Expand the chord indicator into a which-key cheatsheet. Default true. */
    chordHints?: boolean;
    /** Keybinding preset last applied ('vscode' | 'vim' | 'jetbrains'). */
    presetId?: string;
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
    /** Show transient in-app toast popups for new notifications. */
    inAppToasts: boolean;
  };
  claude: {
    /** Default model identity for new agents ('' = Claude Code's own default). */
    defaultModel: string;
    /** Selected token window, separate from the model identity. */
    contextWindow: number | null;
    /** Concrete model ids observed in transcripts, to enrich the spawn dropdown. */
    seenModels: string[];
    /** Default for the spawn dialog's `--dangerously-skip-permissions` toggle. */
    skipPermissionsDefault: boolean;
    /** Permission mode pre-selected in the spawn dialog ('' = the provider's
     *  own default, i.e. 'Ask to approve'). Remembered from the last spawn so a
     *  chosen mode (plan / accept edits) sticks for the next new agent. */
    defaultPermissionMode: string;
    /** Which view a Claude pane opens in by default: rich 'gui' or raw 'terminal'. */
    defaultView: 'gui' | 'terminal';
    /** How runs of tool calls render in the GUI: prose summary 'cards', or the
     *  'trace' waterfall monitor (per-call duration bars + dig-in rows). */
    workLog: 'cards' | 'trace';
    /** Show a small HH:MM stamp on chat turns in the GUI conversation. */
    showTimestamps: boolean;
    /** Render the file contents a Read tool call returned, inline in the
     *  expanded work log. Off = just the "Read <file>" line, no body. */
    showFileReads: boolean;
    /** How new Claude sessions run: 'pty' (the classic Claude Code TUI in a
     *  PTY — Term + GUI) or 'stream' (headless `--print --output-format
     *  stream-json` via claudemon's managed adapter — GUI only). Per-spawn
     *  overridable in the spawn dialog. */
    transport: 'pty' | 'stream';
    /** Experimental: install claudemon's hooks + statusLine into a private
     *  overlay settings file passed to `claude` via `--settings`, instead of
     *  mutating the user's global `~/.claude/settings.json`. Default off. */
    settingsOverlay?: boolean;
    /** Optional per-session cost budgets (USD), keyed by session id. Set from
     *  the inspector; an OS notification fires once when a session's spend
     *  crosses its budget. Absent/0 = no budget. */
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
  /** Codex-specific defaults. The twin of `claude` above, deliberately its own
   *  section rather than a key under `agents`: the transport is a per-harness
   *  fact (each harness has its own two session shapes), and one resolver reads
   *  `config.<provider>.transport` for both — see main/lib/spawnTransport.ts. */
  codex: {
    /** How new Codex sessions run: 'stream' (headless `codex app-server` over
     *  ws via claudemon's managed adapter — GUI only, the shipped default and
     *  the twin of Claude's stream transport) or 'pty' (the hybrid: the native
     *  Codex TUI in a terminal plus the structured GUI on one shared thread).
     *  Per-spawn overridable in the spawn dialog. */
    transport: 'pty' | 'stream';
  };
  /** Defaults applied when spawning a new agent. */
  agents: {
    /** Coding-agent backend pre-selected in the spawn dialog. */
    defaultProvider: string;
    /** Directory the spawn dialog opens at. '' = app launch cwd. */
    defaultCwd: string;
    /** Fleet Manager home directory ('' = derived: the configured projects'
     *  common parent, else $HOME). */
    fleetRoot: string;
    /** Fleet full access: the Fleet Manager runs with permissions bypassed and
     *  its facade token carries the yolo grant, so the workers it dispatches
     *  may run bypassed too. Read (with per-project `yolo`) by
     *  services/fullAccessGrants — the single grant formula the mint path and
     *  the live reconciler share. */
    fleetFullAccess: boolean;
    /** Coding-agent harness the Fleet Manager itself runs on ('' = claude).
     *  The manager needs an MCP client to dispatch at all, so this is
     *  claude/codex/opencode — see SupervisorSection. */
    managerProvider: string;
    /** Coordinator model for the Fleet Manager's OWN conversation, keyed by
     *  harness — `managerProvider` shipped with no model twin, so the manager
     *  always ran on its harness's default. Per-harness because a model id is
     *  not portable (`fable` means nothing to codex), and because switching
     *  `managerProvider` must not destroy the other harness's choice. Read
     *  through lib/roleModels, never directly. */
    managerModels?: Record<string, string>;
    /** Reasoning effort for the Fleet Manager's own conversation, keyed by
     *  harness. Per-harness because the ladders differ per CLI (claude
     *  low..max, codex minimal..xhigh, copilot its own seven); '' / absent =
     *  the harness's own default. Read through lib/roleModels. */
    managerEfforts?: Record<string, string>;
    /** User-configured binary paths per provider. '' = auto-detect on PATH. */
    binaries: {
      claude: string;
      codex: string;
      copilot: string;
      opencode: string;
      pi: string;
    };
    /** Name new agents after their first exchange (see services/agentTitler). */
    autoTitle?: {
      enabled?: boolean;
      /** Legacy single model for the one-shot title call. Ships `'haiku'`, a
       *  CLAUDE alias, so it is honoured only for harnesses that can serve it
       *  ('' = the harness's own default). Superseded by `models`. */
      model?: string;
      /** Per-harness title models, keyed by provider. Unlike the manager’s
       *  map this is not a MEMORY of one picker: every agent is titled by its
       *  OWN harness, so a mixed fleet needs several of these live at once.
       *  Resolved by lib/roleModels `resolveTitleModel`. */
      models?: Record<string, string>;
    };
  };
  /** Directories surfaced in the Overview pane for quick agent launching. */
  directories: {
    recent: string[];
    favourites: string[];
  };
  /** Per-directory script buttons, keyed by workspace root (normalized cwd). */
  scripts: Record<string, ScriptEntry[]>;
  /**
   * Per-directory widget boards, keyed by workspace root (normalized cwd) exactly
   * as `scripts` is. Deliberately here rather than in the hub layout doc: the
   * layout doc is per-AgentWorkspace and is broadcast to every connected client
   * (including the phone mirror), whereas a widget board belongs to a project and
   * several agents can share one cwd.
   */
  widgets: Record<string, WidgetPlacementEntry[]>;
  /**
   * Per-directory project identity, keyed by workspace root (normalized cwd)
   * exactly as `scripts` and `widgets` are (renderer lib/projectKey).
   *
   * Every field is optional because the useful default is DERIVED, not
   * configured: a project with no entry at all still gets a stable mark from
   * its own path, so the fleet is legible before anyone opens a settings page.
   * An entry only records the parts a human chose to override.
   */
  projects: Record<string, ProjectIdentity>;
  apps: AppEntry[];
  /** In-app auto-update (electron-updater over the GitHub Release feed). */
  updates: {
    /** Master switch for auto-update. Default true; only acts in packaged builds. */
    enabled: boolean;
    /** Release channel electron-updater reads ('latest', 'beta', …). */
    channel: string;
  };
  editor: {
    /** How files open: 'codemirror' (legacy value — now the sandboxed editor
     *  plugin, falling back to the OS editor), or your $EDITOR in a 'terminal'. */
    engine: 'codemirror' | 'terminal';
    /** Command for the 'terminal' engine; the file path is appended as its last arg. */
    terminalCommand: string;
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
 * workspace prefix, then the key). Derived from the shared default-config seam
 * (CONFIG_DEFAULTS, ultimately services/hub/cmd/brain/config_defaults.json) so
 * there's one source of truth. Kept in sync with the renderer's DEFAULT_SHORTCUTS
 * (hooks/useConfig.ts).
 */
const DEFAULT_SHORTCUTS: Record<string, string> = { ...CONFIG_DEFAULTS.keybindings.shortcuts };

/**
 * Old nested chord defaults (pre-flattening). A saved shortcut whose value still
 * matches one of these was never customized by the user — it's a stale default —
 * so it's migrated to the new single-key default. Any other value is a genuine
 * user choice and is preserved untouched.
 */
const OLD_CHORD_DEFAULTS: Record<string, string> = {
  'new-terminal': 'prefix n t',
  'new-claude': 'prefix n c',
  'new-browser': 'prefix n b',
  'prev-tab': 'prefix t [',
  'next-tab': 'prefix t ]',
  'move-tab-left': 'prefix t ,',
  'move-tab-right': 'prefix t .',
  'rename-tab': 'prefix t r',
  'close-pane': 'prefix t w',
  split: 'prefix p s',
  'quick-split': 'prefix p c',
  'nav-left': 'prefix p h',
  'nav-down': 'prefix p j',
  'nav-up': 'prefix p k',
  'nav-right': 'prefix p l',
};

function defaultConfig(): Config {
  // Built from the shared default-config seam (CONFIG_DEFAULTS, generated from
  // services/hub/cmd/brain/config_defaults.json that the brain go:embeds) so the
  // desktop and headless-brain defaults are one source of truth. A deep clone
  // keeps callers free to mutate the returned config (deepMerge, migrations).
  const base = structuredClone(CONFIG_DEFAULTS) as unknown as Config;
  // The shared JSON carries the non-Windows shell list; overlay the real
  // platform-aware list here (Git Bash / PowerShell on win32). Everything else —
  // including keybindings.shortcuts — comes straight from the shared defaults.
  base.terminal.shells = defaultShells();
  return base;
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

/**
 * Thrown instead of writing anything when a save names a wholesale path with a
 * value that is not an object.
 *
 * The two writers used to DISAGREE on this exact input, and both answers were
 * wrong. The Go brain coerced it to `{}` — deleting the user's entire projects /
 * customThemes / budgets map and reporting a successful save, with no backup
 * taken on a successful write. This side wrote the bad value through VERBATIM
 * (`dst[leaf] = src[leaf] ?? {}`), leaving a string where a map belongs, so
 * `config.projects[cwd]` came back undefined for every project the user had.
 * Both refuse now, held together by contracts/wholesale-config-paths.json's
 * `valueCases` block.
 */
export class WholesaleValueError extends Error {
  readonly dottedPath: string;
  constructor(dottedPath: string, got: string) {
    super(
      `${dottedPath} is replaced wholesale on save, so it must be sent as an object (got ${got}). ` +
        `Nothing was written — send the full map you want kept, or {} to empty it`,
    );
    this.name = 'WholesaleValueError';
    this.dottedPath = dottedPath;
  }
}

/** What the caller actually sent, for the refusal message. */
function describeWholesaleValue(v: unknown): string {
  if (v === null) return 'null';
  if (Array.isArray(v)) return 'an array';
  if (typeof v === 'string') return 'a string';
  if (typeof v === 'number') return 'a number';
  if (typeof v === 'boolean') return 'a boolean';
  return typeof v;
}

/**
 * Undo the deep merge at one dotted path, so the caller's map replaces the
 * merged one outright.
 *
 * A no-op unless the leaf key is actually PRESENT in `partial` — absence means
 * "I'm not touching this map", which must leave the merged value alone. An
 * explicit `undefined` counts as absent: JavaScript spells "I did not provide
 * this" that way, JSON cannot carry it at all, and the Go twin therefore never
 * sees it.
 *
 * Anything else that is not a plain object is REFUSED (see WholesaleValueError),
 * `null` included. `{}` already says "empty this map", unambiguously and in both
 * languages, so `null` adds no expressive power and is overwhelmingly likelier
 * to be an accident — a serialiser's undefined, a lookup that missed — than an
 * intent to delete everything the user owns. (It used to mean "empty it", via
 * the `?? {}`; nothing in this repo ever sent it.)
 */
export function applyWholesale(
  merged: Record<string, unknown>,
  partial: unknown,
  dottedPath: string,
): void {
  const keys = dottedPath.split('.');
  const leaf = keys.pop() as string;
  let src = partial as Record<string, unknown> | undefined;
  let dst = merged;
  for (const k of keys) {
    src = src?.[k] as Record<string, unknown> | undefined;
    if (!src || typeof src !== 'object') return;
    // The merge already created every parent the partial has, but a partial
    // whose parent is not an object in the target would leave dst undefined.
    const nextDst = dst[k];
    if (!nextDst || typeof nextDst !== 'object') return;
    dst = nextDst as Record<string, unknown>;
  }
  if (!src || !Object.prototype.hasOwnProperty.call(src, leaf)) return;
  const value = src[leaf];
  if (value === undefined) return;
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new WholesaleValueError(dottedPath, describeWholesaleValue(value));
  }
  dst[leaf] = value;
}

// Deep merge source into target, preserving target defaults for missing keys.
// Exported so the cross-language deepMerge contract test (contracts/
// deepmerge-cases.json, also consumed by the Go config.go test) can exercise it
// directly — it's pure, so exporting carries no risk.
export function deepMerge(target: any, source: any): any {
  if (!source || typeof source !== 'object') return target;
  const result = { ...target };
  for (const key of Object.keys(source)) {
    // A null/undefined source value means "unset" (e.g. a bare `ui:` line in
    // YAML parses to { ui: null }). Skip it so the target's default survives
    // instead of being wiped out.
    if (source[key] === null || source[key] === undefined) continue;
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

/** Normalize the Claude selection after a read and before a write. `source`
 *  identifies which half of a partial selection the caller actually supplied,
 *  so changing `defaultModel` alone cannot accidentally inherit the previous
 *  model's window. The historical empty string remains a config-only adapter
 *  for "let Claude choose"; it is not a valid ModelSelection. */
function normalizeClaudeConfigSelection(config: Config, source?: unknown): Config {
  const claude = config.claude as Config['claude'] & Record<string, unknown>;
  const sourceRoot =
    source && typeof source === 'object' ? (source as Record<string, unknown>) : null;
  const sourceClaude =
    sourceRoot?.claude && typeof sourceRoot.claude === 'object' && !Array.isArray(sourceRoot.claude)
      ? (sourceRoot.claude as Record<string, unknown>)
      : null;
  const hasModel =
    !!sourceClaude && Object.prototype.hasOwnProperty.call(sourceClaude, 'defaultModel');
  const hasWindow =
    !!sourceClaude && Object.prototype.hasOwnProperty.call(sourceClaude, 'contextWindow');
  const rawModel = hasModel ? sourceClaude!.defaultModel : claude.defaultModel;
  if (typeof rawModel !== 'string') {
    throw new ModelSelectionError('invalid-model-type', 'claude.defaultModel must be a string');
  }

  if (rawModel.trim() === '') {
    claude.defaultModel = '';
    claude.contextWindow = null;
  } else {
    const rawWindow = hasWindow
      ? sourceClaude!.contextWindow
      : hasModel
        ? undefined
        : claude.contextWindow;
    const selection = normalizeModelSelection(rawModel, rawWindow as number | null | undefined);
    claude.defaultModel = selection.model;
    claude.contextWindow = selection.contextWindow;
  }

  if (Array.isArray(claude.seenModels)) {
    claude.seenModels = Array.from(
      new Set(
        claude.seenModels.flatMap((model) => {
          if (typeof model !== 'string' || !model.trim()) return [];
          try {
            return [normalizeModelSelection(model).model];
          } catch {
            return [];
          }
        }),
      ),
    ).sort();
  }
  return config;
}

/**
 * One-time migration from the old keybindings schema (mode/leader + Ctrl-letter
 * map) to the prefix-forward scheme. The old defaults were written to disk on
 * first run, so without this every existing user would keep the legacy bindings
 * (and their terminal-stealing Ctrl+L/D/S). Resets keybindings wholesale.
 * (It used to preserve Vim keybinding mode as editor.vim; that field died with
 * the in-app CodeMirror editor — nothing reads it.)
 */
function migrateKeybindings(cfg: Config): Config {
  const kb = cfg.keybindings as { mode?: string; leader?: string; prefix?: string } | undefined;
  const isLegacy = !!kb && (kb.mode !== undefined || kb.leader !== undefined || !kb.prefix);
  if (!isLegacy) return cfg;

  // Reset to the shared default keybindings (prefix + preset), not a hardcoded
  // literal, so this stays in lockstep with the Go brain's migrateKeybindings
  // (which resets to defaultConfig()) and the current default preset.
  cfg.keybindings = {
    prefix: CONFIG_DEFAULTS.keybindings.prefix,
    chordHints: true,
    presetId: CONFIG_DEFAULTS.keybindings.presetId,
    shortcuts: { ...DEFAULT_SHORTCUTS },
  };

  try {
    atomicWriteFileSync(getConfigFilePath(), yaml.dump(cfg, { lineWidth: -1 }));
  } catch (err) {
    console.error('[ConfigService] keybindings migration write failed:', err);
  }
  return cfg;
}

/**
 * Second-pass migration for users whose config predates the chord flattening but
 * postdates the schema rewrite (so migrateKeybindings leaves them alone). Any
 * shortcut still holding its exact OLD_CHORD_DEFAULTS value was never touched by
 * the user — it's a stale nested default — so rewrite it to the new flat default.
 * A value that differs from the old default is a real user choice and is kept.
 */
function migrateFlatChords(cfg: Config): Config {
  const shortcuts = cfg.keybindings?.shortcuts;
  if (!shortcuts) return cfg;

  let changed = false;
  for (const [action, oldDefault] of Object.entries(OLD_CHORD_DEFAULTS)) {
    if (shortcuts[action] === oldDefault && DEFAULT_SHORTCUTS[action] !== undefined) {
      shortcuts[action] = DEFAULT_SHORTCUTS[action];
      changed = true;
    }
  }
  if (!changed) return cfg;

  try {
    atomicWriteFileSync(getConfigFilePath(), yaml.dump(cfg, { lineWidth: -1 }));
  } catch (err) {
    console.error('[ConfigService] flat-chord migration write failed:', err);
  }
  return cfg;
}

/**
 * Action ids removed from the app whose bindings linger in real configs: the
 * full shortcuts map was historically persisted wholesale (first-run defaults,
 * the keybindings migration, and Settings rebinds), and the chord tree consumes
 * the record key-by-key — so a removed action would otherwise surface as a dead
 * which-key leaf. Pruned on read and the cleanup written back to disk.
 */
const REMOVED_SHORTCUTS = ['cycle-view'];

function pruneRemovedShortcuts(cfg: Config): Config {
  const shortcuts = cfg.keybindings?.shortcuts;
  if (!shortcuts) return cfg;

  let changed = false;
  for (const action of REMOVED_SHORTCUTS) {
    if (action in shortcuts) {
      delete shortcuts[action];
      changed = true;
    }
  }
  if (!changed) return cfg;

  try {
    atomicWriteFileSync(getConfigFilePath(), yaml.dump(cfg, { lineWidth: -1 }));
  } catch (err) {
    console.error('[ConfigService] removed-shortcut prune write failed:', err);
  }
  return cfg;
}

/** How many times saveConfigLocked retries its compare-and-swap before giving
 *  up. Mirrors briefService.ts's CAS_ATTEMPTS and the Go twin's
 *  saveCASAttempts — the same "an outside writer beat us, recompute against
 *  what's actually there" shape. */
const SAVE_CAS_ATTEMPTS = 5;

/** Runs once per saveConfigLocked attempt, immediately after the merge is
 *  computed and before the CAS check. A no-op in production; tests override it
 *  to inject a write that lands in exactly that window (a non-lock-
 *  participating writer beating us to disk), the same way a real one would. */
export let preWriteHook: () => void = () => {};
export function setPreWriteHookForTest(fn: () => void): void {
  preWriteHook = fn;
}

// Exported so tests can construct a genuinely fresh instance (this.config
// starts undefined) without the fragility of resetting/reimporting the whole
// module — the app itself only ever uses the `configService` singleton below.
export class ConfigService {
  private config: Config;
  /** Notified whenever the effective config changes — our own saves AND writes
   *  by anyone else (the brain serving the web/phone clients, a hand edit). */
  private listeners = new Set<(cfg: Config) => void>();
  private watcher: fs.FSWatcher | null = null;
  private watchTimer: ReturnType<typeof setTimeout> | null = null;
  /** Set when the on-disk config exists but could not be read or parsed. While
   *  true we run on in-memory defaults and REFUSE to write config.yaml — saving
   *  would replace the user's (broken but recoverable) file with defaults.
   *  Cleared on the next successful load (reloadConfig / restart). */
  private persistBlocked = false;
  /** Identity of config.yaml when `this.config` was last loaded: `mtimeMs:size`.
   *  The desktop app is NO LONGER the only writer: the headless brain
   *  (services/hub) serves config.get/save over the hub bus for the web (/app),
   *  mobile (/m) and desktop-bus clients, and writes the *same* config.yaml.
   *  Without this gate, a main-process save (e.g. usageAccumulator recording
   *  seenModels) deep-merges onto this startup cache and clobbers whatever the
   *  brain persisted after launch — user-visible as "settings getting reset".
   *  Mirrors the brain's own gate (services/hub/cmd/brain/config.go).
   *
   *  It carries the SIZE and is compared for INEQUALITY (not ">") because
   *  mtime alone, ordered, cannot see the other writer at all when its save
   *  lands in the same filesystem timestamp tick — 1s granularity on ext4 with
   *  128-byte inodes, HFS+ and NFSv3, 2s on FAT/exFAT. */
  private loadedStamp = '';
  /** Raw bytes of the last config.yaml we backed up as `.broken-*`, so a save
   *  that re-reads an unchanged broken file doesn't mint a backup per call. */
  private lastBrokenBackup: string | null = null;

  constructor() {
    this.config = this.loadFromDisk();
    this.loadedStamp = this.configStamp();
  }

  /** `mtimeMs:size` of config.yaml, or '' when it's absent/unreadable (so a
   *  missing file never looks like a change against a loaded cache). */
  private configStamp(): string {
    try {
      const st = fs.statSync(getConfigFilePath());
      return `${st.mtimeMs}:${st.size}`;
    } catch {
      return '';
    }
  }

  /** Reload from disk when config.yaml changed under us — i.e. the brain (its
   *  own process) wrote it since we last read. mtime-gated, so the steady state
   *  is a single stat with no re-parse. Mirrors configService (Go) get(). */
  private refreshIfChangedOnDisk(): void {
    const s = this.configStamp();
    if (this.config == null || (s !== '' && s !== this.loadedStamp)) {
      this.config = this.loadFromDisk();
      this.loadedStamp = this.configStamp();
    }
  }

  private loadFromDisk(): Config {
    const defaults = defaultConfig();
    const configPath = getConfigFilePath();
    this.persistBlocked = false;

    let data: string;
    try {
      data = fs.readFileSync(configPath, 'utf-8');
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code === 'ENOENT') {
        if (this.config != null) {
          // We had a config a moment ago — this is a mid-run disappearance
          // (e.g. a hand edit that truncated before rewriting), not a first
          // run. Treating it as first run would seed bare defaults and the
          // next save would write them over whatever the user actually has.
          this.persistBlocked = true;
          console.error(
            `[ConfigService] ${configPath} disappeared mid-run — running on the last-known ` +
              'config in memory; saves are disabled until it reappears.',
          );
          return this.config;
        }
        // Genuine first run — no config file yet: seed it with defaults.
        this.writeDefaults();
        return defaults;
      }
      // Transient read failure (EACCES, EBUSY, …): the file exists but we
      // couldn't read it. Run on defaults in memory and never write over a
      // file we couldn't read.
      this.persistBlocked = true;
      console.error(
        `[ConfigService] FAILED TO READ ${configPath} — running on defaults in memory; ` +
          'saves are disabled until the file loads (fix it, then reload):',
        err,
      );
      return defaults;
    }

    try {
      const parsed = yaml.load(data) as Partial<Config>;
      if (parsed == null || typeof parsed !== 'object' || Array.isArray(parsed)) {
        // A 0-byte / whitespace-only / comment-only config.yaml is NOT a parse
        // error — yaml.load returns undefined/null and deepMerge would quietly
        // hand back untouched defaults. Adopting that silently is a total
        // config loss on the next save (saveConfigLocked re-reads via this
        // same function unconditionally). Treat it like the parse-error branch
        // below: block saves rather than let the next write reset the file.
        this.persistBlocked = true;
        console.error(
          `[ConfigService] ${configPath} did not parse to an object — running on defaults ` +
            'in memory; your config file was NOT modified and saves are disabled until it does:',
          parsed,
        );
        return defaults;
      }
      // Retire config blocks whose feature is gone before the merge sees them:
      // deepMerge copies every source key, so an orphan would otherwise be
      // re-serialized by the next save forever. Strictly one key at a time —
      // see ORPHANED_CONFIG_KEYS; this is NOT unknown-key pruning.
      this.pruneOrphanedKeys(parsed as Record<string, unknown>, data, configPath);
      const merged = normalizeClaudeConfigSelection(deepMerge(defaults, parsed) as Config, parsed);
      // migrateKeybindings runs first: a legacy-schema config is reset wholesale
      // to the flat defaults, after which migrateFlatChords is a no-op. A modern
      // config passes migrateKeybindings untouched and migrateFlatChords then
      // upgrades any stale nested-default chords in place. Finally, bindings for
      // actions that no longer exist are pruned.
      this.lastBrokenBackup = null;
      return pruneRemovedShortcuts(migrateFlatChords(migrateKeybindings(merged)));
    } catch (err) {
      if (err instanceof ModelSelectionError) {
        // The YAML is valid. Preserve every unrelated setting and reset only
        // the invalid model pair; never rename/discard a valid config as if it
        // were a syntax failure.
        const parsed = yaml.load(data) as Partial<Config>;
        const merged = deepMerge(defaults, parsed) as Config;
        merged.claude.defaultModel = defaults.claude.defaultModel;
        merged.claude.contextWindow = defaults.claude.contextWindow;
        this.lastBrokenBackup = null;
        console.error(
          `[ConfigService] invalid Claude model selection in ${configPath}; ` +
            'using the shipped model/window default while preserving the rest of the config:',
          err,
        );
        return pruneRemovedShortcuts(migrateFlatChords(migrateKeybindings(merged)));
      }
      // Malformed YAML (e.g. a hand-edit left a syntax error). This must NOT
      // wipe the user's config: back the broken file up, log loudly, run on
      // defaults in memory, and block saves so nothing overwrites the file.
      this.persistBlocked = true;
      console.error(
        `[ConfigService] FAILED TO PARSE ${configPath} — running on defaults in memory; ` +
          'your config file was NOT modified and saves are disabled until it parses:',
        err,
      );
      if (this.lastBrokenBackup !== data) {
        try {
          const backupPath = `${configPath}.broken-${new Date().toISOString().replace(/[:.]/g, '-')}`;
          fs.copyFileSync(configPath, backupPath);
          this.lastBrokenBackup = data;
          console.error(`[ConfigService] backed up the unparseable config to ${backupPath}`);
        } catch (backupErr) {
          console.error('[ConfigService] failed to back up the broken config:', backupErr);
        }
      }
      return defaults;
    }
  }

  /**
   * Delete retired top-level blocks (ORPHANED_CONFIG_KEYS) from the just-parsed
   * config, and persist the tidied file when — and only when — the rewrite is
   * provably nothing but that deletion.
   *
   * The write is a byte-level splice of the text we just read, NOT `yaml.dump`
   * of the parsed object, because dump loses every comment and re-orders keys.
   * The in-memory config is cleaned either way; a file we cannot rewrite safely
   * is simply left alone (the key just stays on disk, harmlessly, as it does
   * today).
   *
   * Like the keybindings migrations above this writes outside the cross-process
   * config lock, and for the same reason: it runs INSIDE loadFromDisk, which
   * saveConfigLocked calls while already holding it. saveConfigLocked's CAS is
   * what covers the resulting window — see its comment.
   */
  private pruneOrphanedKeys(
    parsed: Record<string, unknown>,
    raw: string,
    configPath: string,
  ): void {
    const { removed, text } = pruneOrphanedConfigKeys(parsed, raw);
    if (removed.length === 0) return;
    if (text == null) {
      console.warn(
        `[ConfigService] retired config key(s) ${removed.join(', ')} ignored in memory, but ` +
          `${configPath} was left untouched — the block could not be removed without ` +
          'changing something else in the file.',
      );
      return;
    }
    try {
      atomicWriteFileSync(configPath, text);
      console.log(`[ConfigService] removed retired config key(s): ${removed.join(', ')}`);
    } catch (err) {
      console.error('[ConfigService] retired-key prune write failed:', err);
    }
  }

  private writeDefaults(): void {
    try {
      const data = yaml.dump(defaultConfig(), { lineWidth: -1 });
      atomicWriteFileSync(getConfigFilePath(), data);
    } catch (err) {
      console.error('[ConfigService] failed to write default config:', err);
    }
  }

  getConfig(): Config {
    // Fold in an external (brain) write before handing the config out, so
    // main-process readers (spawn defaults, notifications, …) don't run on a
    // stale startup cache after the web/mobile client changed a setting.
    this.refreshIfChangedOnDisk();
    return this.config;
  }

  reloadConfig(): Config {
    this.config = this.loadFromDisk();
    this.loadedStamp = this.configStamp();
    return this.config;
  }

  saveConfig(partial: Partial<Config>): Config {
    // config.yaml has a second writer in another process (the Go brain, which
    // answers config.save for the web and mobile Settings panes). The refresh →
    // merge → write below is exactly the sequence that must not interleave with
    // theirs: the mtime gate closes the refresh, nothing spans the three. Hold
    // the cross-process lock across all of it — see contracts/config-lock.json.
    try {
      return withConfigLock(getConfigFilePath(), () => this.saveConfigLocked(partial));
    } catch (err) {
      // A malformed wholesale value is REFUSED OUT LOUD, not swallowed into a
      // success carrying the old config. That refusal replaces the branch that
      // used to write the bad value through (and, on the Go twin, empty the
      // map), so the caller learning about it is the entire point — the bus
      // handler in hubCapabilities turns it into an error reply, and the IPC
      // handler rejects the renderer's promise.
      if (err instanceof WholesaleValueError) throw err;
      // Could not take the lock: the other writer is mid-write (or wedged).
      // Writing anyway is the bug this exists to prevent, so refuse — and say
      // so, because the setting the user just changed did not land.
      console.error('[ConfigService] config not saved:', err);
      return this.config;
    }
  }

  /**
   * saveConfig's body, run while holding the cross-process config lock.
   *
   * The lock alone is enough for the two COOPERATING writers (this process and
   * the Go brain, both of which take it): between them, nothing can land
   * between our read and our write. It says nothing about a writer that does
   * not participate — a hand edit of config.yaml, or (inside this very
   * process) loadFromDisk's own one-time keybindings/chord/shortcut
   * migrations, which write directly rather than routing back through
   * saveConfig(). Those are the only remaining window, and this loop closes it
   * the way briefService's appendBriefLine closes the equivalent one for
   * brief.md: re-check immediately before publishing, and if the file moved
   * under us, recompute against what is actually there instead of overwriting
   * it. Mirrors the Go twin (services/hub/cmd/brain/config.go saveLocked).
   */
  private saveConfigLocked(partial: Partial<Config>): Config {
    for (let attempt = 0; attempt < SAVE_CAS_ATTEMPTS; attempt++) {
      // Fold in any external write (the brain editing config.yaml in its own
      // process) BEFORE merging our partial, so a stale in-memory cache can't
      // clobber it. UNCONDITIONALLY, not through the stamp gate: the gate is a
      // cheap-read optimisation for get(), and a write we cannot see is the
      // exact failure it would let through (the other writer's save landing in
      // the same filesystem tick at the same length). Under the cross-process
      // lock this is a genuine read-modify-write, and a save is rare enough to
      // pay one read.
      this.config = this.loadFromDisk();
      this.loadedStamp = this.configStamp();
      // Merge into a LOCAL value, not into this.config. The cache is only
      // adopted once the bytes are on disk — see the write branch below.
      const merged = deepMerge(this.config, partial) as Config;
      // The user-owned maps (themes, budgets, project identities) are sent
      // whole or not at all: when the caller sends one, it IS the truth.
      // Deep-merge can only add or overwrite keys, so under it an entry the
      // user just deleted comes straight back. Undo the merge for those paths
      // and take the caller's map verbatim. The renderer reads the same list
      // to know not to trim them on the way out — see main/shared/configWholesale.
      // A throw here aborts the WHOLE save, above the CAS and above the write:
      // nothing has touched disk yet, so config.yaml is untouched and
      // this.config still describes it. Deliberately not "skip this path and
      // write the rest" — the merge already folded the caller's other keys in,
      // and a partial apply is how a caller learns its save "worked" while one
      // map silently did not move.
      for (const path of WHOLESALE_CONFIG_PATHS) {
        applyWholesale(merged as unknown as Record<string, unknown>, partial, path);
      }
      normalizeClaudeConfigSelection(merged, partial);
      if (this.persistBlocked) {
        // The on-disk config failed to load (unreadable or unparseable): keep
        // the change in memory only. Writing here would replace the user's
        // file with defaults + this partial — permanent loss of everything
        // else in it.
        console.error(
          '[ConfigService] config file failed to load — change kept in memory only, ' +
            'NOT saved to disk (fix or remove the broken config.yaml, then reload).',
        );
        this.config = merged;
        return this.config;
      }
      preWriteHook();
      // COMPARE-AND-SWAP: re-check the file's identity immediately before
      // publishing. A non-lock-participating writer does not block on
      // withConfigLock, so "nobody changed it while we computed the merge" is
      // a claim that has to be checked, not assumed. Same stamp (mtimeMs:size,
      // compared for inequality) getConfig()'s gate already uses.
      if (this.configStamp() !== this.loadedStamp) {
        continue; // moved under us — recompute against the writer that beat us
      }
      try {
        const data = yaml.dump(merged, { lineWidth: -1 });
        atomicWriteFileSync(getConfigFilePath(), data);
      } catch (err) {
        // Do NOT adopt a value that is not on disk — serving it would make
        // the setting look applied until the next restart reverted it, and
        // the caller (Settings, via IPC.CONFIG_SAVE → ConfigContext.setConfig)
        // renders whatever we return as the applied value. Mirrors the Go
        // twin's saveLocked, which returns c.current on a writeConfigYAML
        // error.
        console.error('[ConfigService] failed to save config:', err);
        return this.config;
      }
      this.config = merged;
      // Record our own write's stamp so the next gate check doesn't mistake it
      // for an external change and pointlessly re-read.
      this.loadedStamp = this.configStamp();
      // Includes saves made by main itself (seen models, budgets) — the case
      // the renderer could never see before.
      this.emitChange();
      return this.config;
    }
    // Exhausted retries: something outside the lock is rewriting config.yaml
    // faster than a save can land. Refuse rather than write over whatever it
    // left — the caller sees the value it did not get, same as a lock timeout.
    console.error(
      `[ConfigService] config.yaml is being rewritten outside the lock faster than this save ` +
        `could land (${SAVE_CAS_ATTEMPTS} attempts) — nothing written`,
    );
    return this.config;
  }

  getConfigPath(): string {
    return getConfigFilePath();
  }

  /**
   * Subscribe to config changes; returns an unsubscribe.
   *
   * Starts a file watcher on first use. The watcher is what catches writes that
   * never went through this process — without it a renderer's snapshot only
   * refreshed on its own save, so Settings could show (and re-send) a value the
   * brain replaced hours ago.
   */
  onChange(cb: (cfg: Config) => void): () => void {
    this.listeners.add(cb);
    this.startWatching();
    return () => {
      this.listeners.delete(cb);
    };
  }

  /** Tell subscribers about the current config. Safe to call redundantly. */
  private emitChange(): void {
    for (const cb of this.listeners) {
      try {
        cb(this.config);
      } catch (err) {
        console.error('[ConfigService] change listener failed:', err);
      }
    }
  }

  private startWatching(): void {
    if (this.watcher) return;
    try {
      // Watch the DIRECTORY, not the file: an atomic write replaces the inode,
      // which silently kills a file watch after the first save.
      this.watcher = fs.watch(getConfigDir(), (_event, filename) => {
        if (filename && filename !== path.basename(getConfigFilePath())) return;
        // Editors and atomic writes fire several events per save; settle first.
        if (this.watchTimer) clearTimeout(this.watchTimer);
        this.watchTimer = setTimeout(() => {
          const st = this.configStamp();
          // Our own writes already emitted; only react to someone else's.
          if (st === '' || st === this.loadedStamp) return;
          this.config = this.loadFromDisk();
          this.loadedStamp = st;
          this.emitChange();
        }, 150);
      });
      this.watcher.on('error', (err) => {
        console.warn('[ConfigService] config watch failed:', err.message);
        this.watcher = null;
      });
    } catch (err) {
      // No watcher (unsupported fs, permissions): saves still notify, only
      // external writes go unseen — exactly the old behaviour.
      console.warn('[ConfigService] could not watch config:', (err as Error).message);
    }
  }
}

export const configService = new ConfigService();
