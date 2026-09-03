export type PaneType =
  | 'terminal'
  | 'browser'
  | 'claude'
  | 'settings'
  | 'review'
  | 'plugin'
  | 'plugins'
  | 'overview'
  | 'library'
  | 'analytics'
  | 'ask'
  | 'editor'
  | 'agentwatch'
  | 'agents'
  | 'inspector'
  | 'mdpreview'
  | 'context'
  | 'sessions'
  | 'guide'
  | 'board';

/** Coding-agent backend an agent workspace / agent pane runs.
 *  `undefined` is treated as `'claude'` for backward compatibility with sessions
 *  and config that predate multi-provider support. See docs/multi-agent-providers.md. */
export type AgentProvider = 'claude' | 'codex' | 'copilot' | 'opencode' | 'pi';

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
  /** Claude agent panes only: 'stream' when the session runs on the headless
   *  stream-json transport (no PTY — GUI-only pane). undefined defers to the
   *  session snapshot, then the config default (claude.transport). */
  transport?: 'pty' | 'stream';
  /** Claude session ID to resume (passed as --resume <id> to a NEW process). */
  resumeSessionId?: string;
  /** Claude session ID to attach to as a viewer — the session is already
   *  running in claudemon and we just want to subscribe to its byte stream
   *  without spawning a second process. Mutually exclusive with resumeSessionId. */
  attachSessionId?: string;
  /** Claude panes: this pane attaches to a session with prior history (a
   *  resume, respawn, or boot-time restore), so an empty conversation means
   *  "transcript replay incoming" and the pane shows the fetching state
   *  instead of the new-agent hero. Never set on fresh spawns. */
  expectHistory?: boolean;
  /** Terminal panes only: a command typed into the PTY once it's ready (used by
   *  the per-directory script buttons). */
  initialCommand?: string;
  /** Claude panes only: text to seed the message input with on first mount —
   *  used when spawning an agent from a library prompt/skill. */
  initialPrompt?: string;
  /** Ask panes only: the AgentWorkspace.id this pane is scoped to (limits the
   *  fleet question to that agent's context). Undefined = fleet-wide. */
  scopeAgentId?: string;
  /** Editor panes only: absolute path of the file being edited. */
  filePath?: string;
  /** Plugin panes only: the contributing plugin's id. Lets the pane mint an
   *  ephemeral, agent-cwd-scoped bus token on mount (and revoke it on unmount)
   *  instead of using the broader static per-plugin token. */
  pluginId?: string;
  /** Agent-watch panes only: the claudemon session that OWNS the watched
   *  subagent/workflow (the parent session whose snapshot carries it). */
  watchSessionId?: string;
  /** Agent-watch panes only: what kind of thing is being watched
   *  ('agents' = fleet timeline of all the session's plain subagents). */
  watchKind?: 'subagent' | 'workflow' | 'agents';
  /** Agent-watch panes only: the subagent id or workflow runId being watched
   *  (for 'agents', the sessionId again — one fleet pane per session). */
  watchId?: string;
  /** Inspector panes only: the claudemon session whose live snapshot the pane
   *  renders (plan / flows / agents / files / usage). One pane per session. */
  inspectorSessionId?: string;
  /** Inspector panes only: the target agent's display name, shown as the card
   *  header (the session id alone isn't friendly). */
  inspectorAgentName?: string;
  /** Markdown-preview panes only: absolute path of the previewed file. */
  previewPath?: string;
  /** Markdown-preview panes only: the repo/working dir the file belongs to
   *  (threaded to "Open in editor" so the editor roots at the project). */
  previewCwd?: string;
  /** Markdown-preview panes only: the CANONICAL path main verified at check
   *  time, when the pane was opened through a checked `file:` URL (the
   *  browser-detour dispatch points in App.tsx). Re-sent with every read so
   *  the reader can refuse if the file changed underneath between the check
   *  and the read (a symlink swap). Absent for FileLink's own preview path,
   *  which was never root-confined to begin with. */
  previewCanonicalPath?: string;
  /** Context panes only: the claudemon session whose context inventory the
   *  pane itemizes. One pane per session. */
  contextSessionId?: string;
  /** Context panes only: the target agent's display name for the header. */
  contextAgentName?: string;
  /** Context panes only: section to scroll into view on open (a click on a
   *  specific inspector chip deep-links here), e.g. 'mcp' | 'skills' |
   *  'plugins' | 'agents' | 'memory'. */
  contextFocus?: string;
}

export interface TabConfig {
  id: string;
  title: string;
  panes: PaneConfig[];
  activePaneId: string;
  /** Epoch ms of the tab's last activity (focus / creation / split).
   *  Absent for tabs predating the feature. */
  lastActiveAt?: number;
  /** tmux-style zoom: this pane temporarily fills the tab (others stay
   *  MOUNTED but hidden, so terminals keep their PTYs). tmux semantics: any
   *  structural or focus mutation unzooms first; a dead pane clears it.
   *  Persists through layout save/restore like the rest of the tab. */
  zoomedPaneId?: string;
}

/** Altitude of the workspace. Global (config.panes.viewLevel).
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
  /** Display name — defaults to the basename of `cwd`, renameable. Auto-titling
   *  may replace it (see useAgentAutoTitle) only while both flags below are
   *  unset: a name you typed is yours, and a title is generated once. */
  name: string;
  /** The name came from a human (spawn dialog field or a rename), so nothing
   *  may overwrite it. */
  nameSetByUser?: boolean;
  /** A generated title has already landed for this agent. Persisted with the
   *  layout, so a restart doesn't spend another model call re-titling. */
  autoTitled?: boolean;
  /** The agent-less "Overview" workspace: holds cross-agent / global plugin
   *  panes (e.g. the Agent Dashboard) that don't belong to any single agent.
   *  Always present, pinned first, not spawnable/terminable. */
  global?: boolean;
  /** Working directory. Used as the default cwd for every pane opened here. */
  cwd: string;
  /** Coding-agent backend this workspace runs. undefined ⇒ 'claude' (back-compat
   *  for agents spawned before multi-provider support). */
  provider?: AgentProvider;
  /** Federation: the peer hub this agent's session lives on (from its
   *  snapshot at adoption). Absent = local. Remote agents keep chat/browser/
   *  inspector but lose local cwd-bound panes and local respawn — see
   *  lib/federation.ts. */
  hub?: string;
  /** Claude only: which transport the agent was spawned on ('pty' classic TUI,
   *  'stream' headless stream-json). Re-passed on respawn. undefined = the
   *  config default at spawn time. */
  transport?: 'pty' | 'stream';
  profileId?: string;
  /** Model passed as `--model` at spawn (alias or full id). '' / undefined = Claude default. */
  model?: string;
  /** Canonical requested pair retained across renderer restart/respawn. */
  modelIdentity?: string;
  contextWindow?: number | null;
  /** Harness-specific reasoning-effort level. Re-passed on respawn. */
  effort?: string;
  /** Permission mode: claude default/acceptEdits/plan/bypassPermissions, managed
   *  ask/yolo. Re-passed on respawn (see lib/providerCaps.ts). */
  permissionMode?: string;
  /** Whether this agent was spawned with `--dangerously-skip-permissions`. */
  skipPermissions?: boolean;
  /** Library item ids (kind 'mcp') this agent was spawned with. Re-passed on
   *  respawn so the same servers reload. */
  mcpItemIds?: string[];
  /** Workspacer MCP tool tier granted at spawn (view/triage/operator);
   *  re-applied on respawn so a restart keeps the same grant. */
  toolScope?: 'view' | 'triage' | 'operator';
  /** Plugin ids whose facade tools this agent may use; re-applied on respawn. */
  pluginTools?: string[];
  /** Spawned as THE Fleet Manager (nudge-eligible parent, profile-dispatch
   *  grants, manager skills). Re-passed on respawn so the re-minted token
   *  carries the manager grants (profilesAllowed + the config-resolved yolo
   *  grant) instead of coming back bare. */
  manager?: boolean;
  /** Manager full-access hint recorded at spawn. Advisory on respawn — the
   *  token's actual yolo grant is config-resolved at mint in main
   *  (services/fullAccessGrants), so a frozen value can't resurrect a grant
   *  the user has since revoked. */
  fleetFullAccess?: boolean;
  /** claudemon session id once spawned. Undefined means the agent is stopped
   *  (e.g. the daemon session ended or didn't survive a restart). */
  sessionId?: string;
  /** The session id this agent last held, retained after it stops so a respawn
   *  can `claude --resume <id>` and reopen the prior conversation instead of
   *  starting blank. Doubles as claude's transcript uuid (we pin `--session-id`
   *  at spawn). Cleared once a fresh, non-resumed session takes over. */
  lastSessionId?: string;
  /** The AgentWorkspace.id of the agent this one is nested under in the
   *  sidebar.
   *  If this card later disappears (the manager was explicitly removed, or a
   *  crashed manager was never adopted here at all) `parentId` on any child that
   *  already resolved it goes dangling — SideBar's `rootOf` falls back to
   *  rendering that child as its own root rather than dropping it, which is
   *  also the hook the sidebar's "Unwatched" chip reads. */
  parentId?: string;
  /** Recorded at adopt time: was `parentId`'s target `manager: true` at that
   *  moment? True = the dispatcher was a confirmed Fleet Manager, not just any
   *  parent. Only meaningful once `parentId` has gone dangling (see above) —
   *  it lets the orphan chip say "its manager ended" instead of the weaker
   *  "its parent is gone" when the stronger claim is actually known. */
  dispatchedByManager?: boolean;
  /** Per-agent tabs. Mirrors what a flat workspace used to hold globally. */
  tabs: TabConfig[];
  activeTabId: string;
}
