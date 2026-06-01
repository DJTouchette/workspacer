export type PaneType = 'terminal' | 'browser' | 'notes' | 'agent' | 'claude' | 'settings' | 'tracker' | 'devops' | 'review' | 'agent-manager' | 'devdaemon';

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
  /** Working directory. Used as the default cwd for every pane opened here. */
  cwd: string;
  profileId?: string;
  /** claudemon session id once spawned. Undefined means the agent is stopped
   *  (e.g. the daemon session ended or didn't survive a restart). */
  sessionId?: string;
  /** Per-agent tabs. Mirrors what a flat workspace used to hold globally. */
  tabs: TabConfig[];
  activeTabId: string;
}
