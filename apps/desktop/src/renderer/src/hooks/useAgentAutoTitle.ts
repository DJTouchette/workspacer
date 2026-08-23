import { useEffect, useRef } from 'react';
import { resolveProvider, type AgentWorkspace } from '../types/pane';
import type { ClaudeSessionSnapshot, ConversationTurn } from '../types/claudeSession';

/**
 * Names each agent after its first exchange, the way a chat service names a
 * conversation — the card stops being another "workspacer" and starts saying
 * what it's for.
 *
 * Deliberately once per agent: the trigger fires when the opening user message
 * has an answer, and the agent is marked titled whether or not a title came
 * back, so a fleet of ten agents costs ten cheap calls for the life of the
 * session rather than one per snapshot.
 *
 * The title itself is generated in the main process (services/agentTitler);
 * this hook only decides WHEN, and what text to hand over.
 */

/** Turns that aren't the human opening the conversation. */
function isRealUserTurn(t: ConversationTurn): boolean {
  return t.role === 'user' && !t.command && !!t.content?.trim();
}

export interface AutoTitleDeps {
  agents: AgentWorkspace[];
  snapshotBySession: Record<string, ClaudeSessionSnapshot>;
  /** Enabled state from config (`agents.autoTitle.enabled`; absent = on). */
  enabled: boolean;
  /** Applies the result — marks the agent titled even for a null title. */
  onTitle: (agentId: string, title: string | null) => void;
}

/**
 * The opening exchange of a conversation: the first genuine user message and
 * the first assistant text that follows it. Null until both exist — an answer
 * is what tells us the agent understood the ask, and it sharpens vague openers
 * ("continue", "fix it") that would make a useless title on their own.
 *
 * Exported for tests.
 */
export function openingExchange(
  conversation: ConversationTurn[] | undefined,
): { userMessage: string; assistantReply?: string } | null {
  if (!conversation?.length) return null;
  const userIdx = conversation.findIndex(isRealUserTurn);
  if (userIdx === -1) return null;
  const reply = conversation
    .slice(userIdx + 1)
    .find((t) => t.role === 'assistant' && !!t.content?.trim());
  if (!reply) return null;
  return {
    userMessage: conversation[userIdx].content!.trim(),
    assistantReply: reply.content?.trim(),
  };
}

/** Agents eligible for a title right now: live, unnamed by a human, untitled. */
export function agentsAwaitingTitle(
  agents: AgentWorkspace[],
  snapshotBySession: Record<string, ClaudeSessionSnapshot>,
  /** Openings already banked from an uncompacted snapshot, keyed by session. */
  remembered?: Map<string, { userMessage: string; assistantReply?: string }>,
): Array<{ agent: AgentWorkspace; exchange: { userMessage: string; assistantReply?: string } }> {
  const out = [];
  for (const agent of agents) {
    if (agent.global || !agent.sessionId) continue;
    if (agent.nameSetByUser || agent.autoTitled) continue;
    const snap = snapshotBySession[agent.sessionId];
    // A compacted snapshot (conversationOffset > 0) has dropped its leading
    // turns, so its "first" user message is just the oldest one still in the
    // window — titling from it would name a resumed session after whatever it
    // was doing an hour ago.
    //
    // But every promoted snapshot goes through compactClaudeSnapshotForBackground,
    // which caps the conversation at 12 turns — so an agent whose opening burst
    // runs past that was never titled at all, permanently, since the offset only
    // ever grows. Bank the opening the first time we see an uncompacted snapshot
    // and title from that; the guard keeps its meaning (never title from a window
    // that lost its true first turn) without losing the chance to a fast start.
    const compacted = (snap?.conversationOffset ?? 0) > 0;
    const banked = remembered?.get(agent.sessionId);
    const exchange = compacted ? banked : (openingExchange(snap?.conversation) ?? banked);
    if (exchange) {
      if (!compacted && remembered && !banked) remembered.set(agent.sessionId, exchange);
      out.push({ agent, exchange });
    }
  }
  return out;
}

export function useAgentAutoTitle({
  agents,
  snapshotBySession,
  enabled,
  onTitle,
}: AutoTitleDeps): void {
  // Agent ids with a call in flight. Not state: this must gate the NEXT effect
  // run, which happens on the very next snapshot — well before the round-trip
  // returns and marks the agent titled.
  const inFlightRef = useRef<Set<string>>(new Set());
  // Openings seen while the snapshot was still whole. Compaction is one-way, so
  // without this an agent that opened fast could never be titled.
  const openingsRef = useRef<Map<string, { userMessage: string; assistantReply?: string }>>(
    new Map(),
  );

  useEffect(() => {
    if (!enabled) return;
    const pending = agentsAwaitingTitle(agents, snapshotBySession, openingsRef.current);
    for (const { agent, exchange } of pending) {
      if (inFlightRef.current.has(agent.id)) continue;
      inFlightRef.current.add(agent.id);
      window.electronAPI
        // The agent's OWN provider titles it: a codex agent must not need a
        // claude binary on PATH to get a name.
        .agentSuggestTitle({ ...exchange, provider: resolveProvider(agent.provider) })
        .then((title) => onTitle(agent.id, title))
        // A cosmetic feature must not spam the console on a missing binary;
        // main already logs the reason once.
        .catch(() => onTitle(agent.id, null))
        .finally(() => inFlightRef.current.delete(agent.id));
    }
  }, [agents, snapshotBySession, enabled, onTitle]);
}
