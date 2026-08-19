/**
 * Agent-card identity helpers for cross-client convergence.
 *
 * The layout is mirrored across clients (desktop, web, phone) through the hub's
 * shared document, last-writer-wins. The real identity of an agent card is its
 * live daemon `sessionId` — but historically each client minted a random card
 * id (`agent-<time>-<n>`), so the *same* session got a *different* id on every
 * client that spawned or auto-adopted it. With a blind whole-array sync that has
 * no dedup, those divergent ids ping-pong and the agent list grows (the classic
 * "spawn one, end up with seven").
 *
 * Two defenses live here:
 *   1. `agentIdForSession` — a deterministic id derived from the sessionId, so
 *      every client produces the SAME card id for a session and never diverges.
 *   2. `dedupeBySessionId` — a safety net applied whenever a layout is taken in,
 *      collapsing any cards that still share a sessionId (e.g. a respawn that
 *      changed the session under an existing card) down to one.
 */
import type { AgentWorkspace } from '../types/pane';

/** Deterministic card id for a card bound to a live daemon session. */
export function agentIdForSession(sessionId: string): string {
  return `agent-${sessionId}`;
}

/**
 * Collapse cards that point at the same `sessionId` into a single card. Two
 * cards for one live session is never valid. The survivor is chosen
 * deterministically (lexicographically-smallest id) so every client agrees on
 * which one to keep, and it is placed at the position of the first occurrence so
 * ordering stays stable. Cards without a sessionId (stopped/local agents) are
 * always kept as-is.
 */
/**
 * Collapse cards that share a card `id` into one. Duplicate ids are the worst
 * corruption this layer can carry: React sibling keys collide (one workspace's
 * pane subtree renders under another's slot — the "Overview is suddenly an
 * agent pane" bug) and `mutateAgent`, which maps by id, writes into every
 * twin. Ids collide when a mid-respawn card (sessionId nulled by a stray
 * eviction tick) met an auto-adopted card minted for the same session — both
 * spell `agent-<sessionId>`. The survivor is the one still holding a live
 * sessionId (the healthy copy); tie goes to the first occurrence.
 */
export function dedupeByCardId(agents: AgentWorkspace[]): AgentWorkspace[] {
  const survivorFor = new Map<string, AgentWorkspace>();
  const result: AgentWorkspace[] = [];

  for (const a of agents) {
    const existing = survivorFor.get(a.id);
    if (!existing) {
      survivorFor.set(a.id, a);
      result.push(a);
    } else if (a.sessionId && !existing.sessionId) {
      const idx = result.indexOf(existing);
      if (idx !== -1) result[idx] = a;
      survivorFor.set(a.id, a);
    }
    // else: duplicate id with no better claim — drop it.
  }

  return result;
}

export function dedupeBySessionId(agents: AgentWorkspace[]): AgentWorkspace[] {
  const survivorFor = new Map<string, AgentWorkspace>();
  const result: AgentWorkspace[] = [];

  for (const a of agents) {
    if (!a.sessionId) {
      result.push(a);
      continue;
    }
    const existing = survivorFor.get(a.sessionId);
    if (!existing) {
      survivorFor.set(a.sessionId, a);
      result.push(a);
    } else if (a.id < existing.id) {
      // Keep the deterministically-smaller id; replace the one already placed.
      const idx = result.indexOf(existing);
      if (idx !== -1) result[idx] = a;
      survivorFor.set(a.sessionId, a);
    }
    // else: a is a duplicate with a larger id — drop it.
  }

  return result;
}
