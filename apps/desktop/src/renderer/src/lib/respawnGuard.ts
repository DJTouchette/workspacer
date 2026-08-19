/**
 * Session ids currently mid-respawn (close → spawnClaude → record re-attach).
 *
 * A respawn REUSES the session id, and the window between closing the old
 * process and committing the new sessionId onto the agent record is where the
 * "bunch of agents" bug lived: a late eviction tick could null the card's
 * sessionId (stopAgentForSession) and the auto-adopt effect — seeing a live
 * session no card owns — minted a SECOND card with the identical deterministic
 * id (`agent-<sessionId>`), after which every id-keyed mutation wrote into
 * both and React's sibling keys collided (the "Overview pane is a random agent
 * pane" corruption). While an id is marked here, eviction and adoption both
 * stand down.
 *
 * Module-level for the same reason as lib/terminatedSessions.ts: the respawn
 * paths live in useAgentManager while eviction and adoption live in
 * useSessionSnapshots and App.
 */
const respawning = new Set<string>();

export function markRespawning(sessionId: string | undefined): void {
  if (sessionId) respawning.add(sessionId);
}

export function isRespawning(sessionId: string): boolean {
  return respawning.has(sessionId);
}

/**
 * Release the guard a beat AFTER the respawn commits. The commit is a setState
 * whose result the adopt effect only sees post-render, and stray teardown
 * ticks from the dying process can be dispatched after the spawn IPC resolves
 * — so a synchronous clear would reopen the window it exists to close. The
 * delay only lengthens how long eviction/adoption stand down for an id the
 * respawn just re-attached; both are no-ops for a healthy card.
 */
export function settleRespawning(sessionId: string | undefined, delayMs = 2000): void {
  if (!sessionId) return;
  setTimeout(() => respawning.delete(sessionId), delayMs);
}

/** Test hook — module state would otherwise leak between cases. */
export function resetRespawnGuard(): void {
  respawning.clear();
}
