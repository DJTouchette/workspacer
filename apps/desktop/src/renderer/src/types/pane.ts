export type PaneType = 'terminal' | 'browser' | 'notes' | 'claude' | 'settings' | 'review' | 'plugin' | 'plugins' | 'overview' | 'library' | 'analytics' | 'ask' | 'editor';

/** Coding-agent backend an agent workspace / agent pane runs.
 *  `undefined` is treated as `'claude'` for backward compatibility with sessions
 *  and config that predate multi-provider support. See docs/multi-agent-providers.md. */
export type AgentProvider = 'claude' | 'codex' | 'opencode' | 'pi';

/** Normalize a possibly-undefined provider to the concrete default ('claude'). */
export function resolveProvider(p: AgentProvider | undefined): AgentProvider {
  return p ?? 'claude';
}

export interface PaneConfig {
  id: string;
  type: PaneType;
  title: string;
  shell?: string;
  cwd?: string;
  url?: string;
  appMode?: boolean;
  hibernated?: boolean;
  profileId?: string;
  /** Coding-agent backend for a 'claude'-type agent pane. undefined ⇒ 'claude'. */
  provider?: AgentProvider;
  /** Claude session ID to resume (passed as --resume <id> to a NEW process). */
  resumeSessionId?: string;
  /** Claude session ID to attach to as a viewer — the session is already
   *  running in claudemon and we just want to subscribe to its byte stream
   *  without spawning a second process. Mutually exclusive with resumeSessionId. */
  attachSessionId?: string;
  /** Terminal panes only: a command typed into the PTY once it's ready (used by
   *  the per-directory script buttons). */
  initialCommand?: string;
  /** Claude panes only: text to seed the message input with on first mount —
   *  used when spawning an agent from a library prompt/skill. */
  initialPrompt?: string;
  /** Ask panes only: the AgentWorkspace.id this pane is scoped to (limits the
   *  supervisor question to that agent's context). Undefined = fleet-wide. */
  scopeAgentId?: string;
  /** Notes panes only: the markdown scratchpad content, persisted with the session. */
  notes?: string;
  /** Editor panes only: absolute path of the file being edited. */
  filePath?: string;
  /** Plugin panes only: the contributing plugin's id. Lets the pane mint an
   *  ephemeral, agent-cwd-scoped bus token on mount (and revoke it on unmount)
   *  instead of using the broader static per-plugin token. */
  pluginId?: string;
}

/** Position + size of a tab's card on the spatial canvas, in world coordinates
 *  (pre-zoom). Only used when the global view mode is 'spatial'; absent until the
 *  card is first placed/dragged, at which point a default grid slot is persisted. */
export interface CanvasRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface TabConfig {
  id: string;
  title: string;
  panes: PaneConfig[];
  activePaneId: string;
  /** Spatial-canvas placement. See {@link CanvasRect}. */
  canvas?: CanvasRect;
  /** Epoch ms of the tab's last activity (focus / creation / split).
   *  Absent for tabs predating the feature. */
  lastActiveAt?: number;
}

/** How the workspace lays out tabs. Global (config.panes.viewMode).
 *  - 'tabs':     the classic horizontal scroll strip (one tab on screen at a time)
 *  - 'spatial':  every tab is a free-floating card on a pannable/zoomable canvas
 *  - 'stacked':  cards in a vertical feed (natural order); wraps top↔bottom */
export type ViewMode = 'tabs' | 'spatial' | 'stacked';

/** Altitude of the workspace, orthogonal to {@link ViewMode}. Global
 *  (config.panes.viewLevel).
 *  - 'piloting': you're inside one agent's workspace (the classic view)
 *  - 'fleet':    the Fleet Deck — a cross-agent radar of live agent cards */
export type ViewLevel = 'fleet' | 'piloting';

/**
 * An agent workspace = one long-lived Claude Code (claudemon) session plus its
 * own set of tabs/panes. The session is identified by `cwd` (+ optional name)
 * and lives in the daemon independent of any UI pane: it is created via
 * `spawnAgent` and only torn down by an explicit `terminateAgent`. Navigating
 * between agents never spawns or kills a session.
 */
export interface AgentWorkspace {
  id: string;
  /** Display name — defaults to the basename of `cwd`, renameable. */
  name: string;
  /** The agent-less "Overview" workspace: holds cross-agent / global plugin
   *  panes (e.g. the Agent Dashboard) that don't belong to any single agent.
   *  Always present, pinned first, not spawnable/terminable. */
  global?: boolean;
  /** Working directory. Used as the default cwd for every pane opened here. */
  cwd: string;
  /** Coding-agent backend this workspace runs. undefined ⇒ 'claude' (back-compat
   *  for agents spawned before multi-provider support). */
  provider?: AgentProvider;
  profileId?: string;
  /** Model passed as `--model` at spawn (alias or full id). '' / undefined = Claude default. */
  model?: string;
  /** Whether this agent was spawned with `--dangerously-skip-permissions`. */
  skipPermissions?: boolean;
  /** Library item ids (kind 'mcp') this agent was spawned with. Re-passed on
   *  respawn so the same servers reload. */
  mcpItemIds?: string[];
  /** claudemon session id once spawned. Undefined means the agent is stopped
   *  (e.g. the daemon session ended or didn't survive a restart). */
  sessionId?: string;
  /** The session id this agent last held, retained after it stops so a respawn
   *  can `claude --resume <id>` and reopen the prior conversation instead of
   *  starting blank. Doubles as claude's transcript uuid (we pin `--session-id`
   *  at spawn). Cleared once a fresh, non-resumed session takes over. */
  lastSessionId?: string;
  /** Marks a supervisor agent — spawned with the workspacer MCP facade so it can
   *  observe and coordinate the other agents. Rendered nested under its parent. */
  kind?: 'supervisor';
  /** For supervisors: the AgentWorkspace.id of the agent this one supervises.
   *  Used to render it nested in the sidebar. Undefined = fleet-level supervisor. */
  parentId?: string;
  /** Per-agent tabs. Mirrors what a flat workspace used to hold globally. */
  tabs: TabConfig[];
  activeTabId: string;
}
