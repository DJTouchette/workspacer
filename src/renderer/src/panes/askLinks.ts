import { AgentWorkspace } from '../types/pane';

export interface SessionRef {
  /** The raw session id from the `session:<id>` token. */
  sessionId: string;
  /** Resolved AgentWorkspace.id, if an agent with this sessionId exists. */
  agentId?: string;
  /** Resolved AgentWorkspace.name, if an agent with this sessionId exists. */
  agentName?: string;
  /** Character index of the start of the match in the original string (includes "session:"). */
  start: number;
  /** Character index one past the end of the match. */
  end: number;
}

/**
 * Find all `session:<id>` tokens in `text` and resolve them against the given
 * fleet.  The session id portion is matched as one or more word characters plus
 * hyphens (covers both short slugs and full UUIDs).
 */
export function findSessionRefs(text: string, agents: AgentWorkspace[]): SessionRef[] {
  // Build a lookup from sessionId → AgentWorkspace for O(1) resolution.
  const bySession = new Map<string, AgentWorkspace>();
  for (const agent of agents) {
    if (agent.sessionId) {
      bySession.set(agent.sessionId, agent);
    }
  }

  const pattern = /session:([\w-]+)/g;
  const refs: SessionRef[] = [];

  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    const sessionId = match[1];
    const agent = bySession.get(sessionId);
    refs.push({
      sessionId,
      agentId: agent?.id,
      agentName: agent?.name,
      start: match.index,
      end: match.index + match[0].length,
    });
  }

  return refs;
}
