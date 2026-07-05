import type { AttentionItem, AttentionKind } from '../types/attention';
import type { SessionAmbientState } from '../types/claudeSession';

/**
 * The Attention Router — a pure scorer/sorter shared by BOTH the Triage Inbox
 * order and the Fleet Deck card buoyancy. No side effects; unit-testable.
 *
 * The single rule of the paradigm: things that block a human outrank things
 * that are merely happening. Within a tier, older items float up (you should
 * clear the thing that's been waiting longest first).
 */
export const KIND_PRIORITY: Record<AttentionKind, number> = {
  approval: 100,
  question: 95,
  error: 80,
  stuck: 70,
  bigdiff: 40,
  done: 20,
};

/** Sort attention items most-urgent first (priority desc, then oldest first). */
export function sortItems(items: AttentionItem[]): AttentionItem[] {
  return [...items].sort((a, b) => {
    if (b.priority !== a.priority) return b.priority - a.priority;
    return a.createdAt - b.createdAt;
  });
}

/**
 * How much an agent — independent of any concrete inbox item — wants your eyes,
 * used to order/buoy Fleet Deck cards. Blocked agents first, then working, then
 * idle, then stopped. `topItemPriority` (the agent's most-urgent open item, if
 * any) dominates so a card with a live approval always outranks a bare state.
 */
export function agentAttentionScore(
  state: SessionAmbientState | undefined,
  topItemPriority: number,
): number {
  if (topItemPriority > 0) return 1000 + topItemPriority;
  switch (state) {
    case 'waiting_approval':
      return 900;
    case 'waiting_input':
      return 880;
    case 'thinking':
      return 500;
    case 'streaming':
      return 480;
    case 'idle':
      return 200;
    default:
      return 0; // stopped / unknown
  }
}
