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
  kind: 'subagent' | 'workflow';
  /** Subagent id or workflow runId. */
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
