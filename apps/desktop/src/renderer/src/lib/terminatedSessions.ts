/**
 * Session ids the user explicitly terminated this app run.
 *
 * Termination is not instantaneous on the daemon side: after `claudeClose`
 * the dying session still emits a few updates (teardown hooks, final
 * statusline ticks) before it reports `ended`. Any of those ticks would
 * re-insert the session into App's promoted snapshot map, where the
 * auto-adopt effect — seeing a "live" session no agent owns — would resurrect
 * the card the user just closed. This tombstone set lets every consumer
 * (snapshot promotion, wholesale refresh, auto-adopt) treat a terminated
 * session as already dead during that window.
 *
 * Module-level on purpose: `terminateAgent` lives in useAgentManager while
 * the snapshot maps live in App, and close-tab/close-pane call terminate
 * directly. Ids are never reused (spawns and resumes pin fresh UUIDs), so
 * entries can live for the rest of the app run.
 */
const terminated = new Set<string>();

export function markSessionTerminated(sessionId: string | undefined): void {
  if (sessionId) terminated.add(sessionId);
}

export function wasSessionTerminated(sessionId: string): boolean {
  return terminated.has(sessionId);
}

/**
 * Lift the tombstone for a single id. A restart-with-settings resume REUSES the
 * old session id (claudeSpawn/managedSpawn: `opts.resumeSessionId || randomUUID()`),
 * so once the resumed session is live again it must be cleared, or every snapshot
 * tick for it gets dropped by App's `wasSessionTerminated` guard and the running
 * agent renders as Stopped forever.
 */
export function clearSessionTerminated(sessionId: string | undefined): void {
  if (sessionId) terminated.delete(sessionId);
}

/** Test hook — module state would otherwise leak between cases. */
export function resetTerminatedSessions(): void {
  terminated.clear();
}
