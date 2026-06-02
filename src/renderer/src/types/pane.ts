export type PaneType = 'terminal' | 'browser' | 'notes' | 'agent' | 'claude' | 'settings' | 'review' | 'plugin' | 'plugins' | 'overview' | 'library';

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
}

export interface TabConfig {
  id: string;
  title: string;
  panes: PaneConfig[];
  activePaneId: string;
}

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
  profileId?: string;
  /** Model passed as `--model` at spawn (alias or full id). '' / undefined = Claude default. */
  model?: string;
  /** Whether this agent was spawned with `--dangerously-skip-permissions`. */
  skipPermissions?: boolean;
  /** claudemon session id once spawned. Undefined means the agent is stopped
   *  (e.g. the daemon session ended or didn't survive a restart). */
  sessionId?: string;
  /** Per-agent tabs. Mirrors what a flat workspace used to hold globally. */
  tabs: TabConfig[];
  activeTabId: string;
}
