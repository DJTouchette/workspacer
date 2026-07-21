/**
 * Cross-component bus for opening live "watch" surfaces.
 *
 * Two shapes, same pattern as reviewBus/workflowBus (a pane rendered deep in
 * some agent's tree has no access to the tab manager, so it dispatches a
 * window CustomEvent that App — which does — handles):
 *
 *  - requestAgentWatch:   open an `agentwatch` pane focused on ONE subagent or
 *    workflow run of a session (from the inspector rail's Agents/Flows tabs).
 *  - requestSessionWatch: open a GUI Claude pane attached as a viewer to a
 *    whole session (from the Agents fleet pane), so you can watch that agent
 *    without leaving the current workspace.
 */

import type { AgentProvider } from '../types/pane';

export const AGENT_WATCH_EVENT = 'agentwatch:open';
export const SESSION_WATCH_EVENT = 'agentwatch:open-session';

export interface AgentWatchTarget {
  /** The claudemon session that owns the watched subagent/workflow. */
  sessionId: string;
  /** 'agents' = fleet timeline of ALL the session's plain subagents. */
  kind: 'subagent' | 'workflow' | 'agents';
  /** Subagent id, workflow runId, or (for 'agents') the sessionId again. */
  id: string;
  /** Tab/pane title for the new watch pane. */
  title: string;
}

export interface SessionWatchTarget {
  sessionId: string;
  cwd?: string;
  title: string;
  provider?: AgentProvider;
}

/** Open a watch pane for one subagent / workflow run. */
export function requestAgentWatch(target: AgentWatchTarget): void {
  window.dispatchEvent(new CustomEvent(AGENT_WATCH_EVENT, { detail: target }));
}

/** Open a GUI viewer pane attached to a whole session. */
export function requestSessionWatch(target: SessionWatchTarget): void {
  window.dispatchEvent(new CustomEvent(SESSION_WATCH_EVENT, { detail: target }));
}

export const INSPECTOR_OPEN_EVENT = 'inspector:open';

/** Open a standalone Inspector pane bound to one session's live snapshot. */
export interface InspectorTarget {
  /** The claudemon session whose snapshot the pane renders. */
  sessionId: string;
  /** Target agent's display name (card header + pane title). */
  agentName?: string;
}

/** Open (or focus) an Inspector pane for a session — from the command palette or
 *  a Fleet Deck card's "Open as pane" action. */
export function requestInspector(target: InspectorTarget): void {
  window.dispatchEvent(new CustomEvent(INSPECTOR_OPEN_EVENT, { detail: target }));
}

export const CONTEXT_OPEN_EVENT = 'context:open';

/** Open a Context pane itemizing what occupies a session's context window. */
export interface ContextTarget {
  /** The claudemon session whose inventory the pane renders. */
  sessionId: string;
  /** Target agent's display name (header + pane title). */
  agentName?: string;
  /** Section to scroll to on open: 'mcp' | 'skills' | 'plugins' | 'agents' | 'memory'. */
  focus?: string;
}

/** Open (or focus) a Context pane for a session — from the inspector rail's
 *  Usage chips or the command palette. */
export function requestContextPane(target: ContextTarget): void {
  window.dispatchEvent(new CustomEvent(CONTEXT_OPEN_EVENT, { detail: target }));
}

export const AGENT_HANDOFF_EVENT = 'agent:handoff';

/** Spawn a successor agent primed with a handoff brief (any → any provider). */
export interface HandoffTarget {
  /** Provider the successor session runs on. */
  targetProvider: AgentProvider;
  /** Working directory the work lives in (successor spawns here too). */
  cwd?: string;
  /** Absolute path of the persisted brief under ~/.workspacer/handoffs/. */
  briefPath: string;
  /** Session being handed off, for the successor pane's title. */
  sourceSessionId: string;
}

/** Ask App to spawn the successor agent for a handoff. */
export function requestHandoff(target: HandoffTarget): void {
  window.dispatchEvent(new CustomEvent(AGENT_HANDOFF_EVENT, { detail: target }));
}

export const RECENT_SESSIONS_REFRESH_EVENT = 'recent-sessions:refresh';

/** Ask the sidebar's RECENT list to refetch now instead of waiting for its
 *  60s poll. Fired after a terminate: the daemon flips the dying session to a
 *  resumable Stopped row a moment later, and without a nudge the row stays
 *  invisible until the next poll tick. */
export function requestRecentSessionsRefresh(): void {
  window.dispatchEvent(new CustomEvent(RECENT_SESSIONS_REFRESH_EVENT));
}
