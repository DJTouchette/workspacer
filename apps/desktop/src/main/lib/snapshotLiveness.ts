/**
 * Is a process running in this session's directory RIGHT NOW?
 *
 * This is lifecycle/locality classification, separate from sidebar curation. A
 * stopped agent can still deserve a card, while a remote peer's cwd must never
 * be mistaken for a directory on this machine.
 *
 * Desktop and headless both use the result when projecting local live-session
 * directories into compatibility context. Ambient agent filesystem access does
 * not depend on this predicate.
 *
 * The clauses are the Go twin's, in the same order and for the same reasons:
 *
 *   - a row that will not decode is not live;
 *   - mode "unknown" is not live. A `terminals.create` SHELL sits in "unknown"
 *     for its whole life and never leaves it, and capspec deliberately leaves
 *     a terminal is not an agent session;
 *   - stopped is not live, in either spelling (claudemon's `mode`, or the
 *     desktop-shaped `status: 'ended'`);
 *   - archived is not live.
 *
 * An empty mode with no ended status IS live: that is the desktop-shaped row
 * carrying its state in `status`.
 *
 * TWIN: snapshotLive in services/hub/cmd/brain/visibility.go.
 */
export function snapshotIsLocalLiveSession(snap: unknown): boolean {
  if (typeof snap !== 'object' || snap === null || Array.isArray(snap)) return false;
  const row = snap as Record<string, unknown>;
  // Read defensively rather than off ClaudeSessionSnapshot: the store also
  // carries claudemon-shaped rows through the enrich overlay, and a field of the
  // wrong TYPE is the "will not decode" clause.
  // FEDERATION: a remote session's cwd is a path on the PEER machine and must
  // never be reported as a local live-session directory. The Go twin has no
  // equivalent clause because the
  // brain derives its cwds from the local claudemon's sessions, which are
  // never hub-stamped.
  if (row.hub !== undefined && (typeof row.hub !== 'string' || row.hub !== '')) return false;
  if (row.mode !== undefined && typeof row.mode !== 'string') return false;
  if (row.status !== undefined && typeof row.status !== 'string') return false;
  if (row.archived !== undefined && typeof row.archived !== 'boolean') return false;
  const mode = (row.mode as string | undefined) ?? '';
  const status = (row.status as string | undefined) ?? '';
  if (mode === 'unknown' || row.archived === true) return false;
  return !(mode === 'stopped' || (mode === '' && status === 'ended'));
}
