/**
 * Is a process running in this session's directory RIGHT NOW?
 *
 * This is the liveness half of the session rule, without the curation half —
 * "should the fs.* allow-list contain this cwd", not "should the sidebar draw a
 * card for it". The two are deliberately different questions: a stopped agent
 * still deserves a card, and must NOT still hand out a filesystem root.
 *
 * It exists because the desktop's `workspaceRoots()` needs exactly this and had
 * nothing. `services/hub/cmd/brain/visibility.go` grew `snapshotLive` for the
 * brain's `agentCwds()` and states the rule as "the fs.* allow-list is a grant,
 * not a view: a stopped agent's directory has to leave it (that is the entire
 * justification for cwdCacheTTL being seconds)". The desktop copy iterated every
 * snapshot with no liveness test at all, so the SAME session row granted an
 * fs root on one provider and was refused by the other — and the desktop's only
 * removal path is a 30-second timer armed by a SessionEnd hook, so a PTY killed
 * without one (SIGKILL, crash, OOM) kept `status: 'active'` and kept its cwd as
 * an fs.* root for the life of the app process. Four capabilities the method
 * table marks `providers: ["main"]` — git.diff, fs.readImage, fs.watch,
 * fs.unwatch — are answered HERE even under the default delegation, so that
 * divergence ships.
 *
 * The clauses are the Go twin's, in the same order and for the same reasons:
 *
 *   - a row that will not decode is not live (a snapshot shape change must never
 *     widen an allow-list);
 *   - mode "unknown" is not live. A `terminals.create` SHELL sits in "unknown"
 *     for its whole life and never leaves it, and capspec deliberately leaves
 *     that method's `cwd` unconfined — holding terminals.create IS the grant. So
 *     counting a shell's cwd as an agent cwd let any caller who could open a
 *     terminal at `/` hand itself `/` as an fs.read root;
 *   - stopped is not live, in either spelling (claudemon's `mode`, or the
 *     desktop-shaped `status: 'ended'`);
 *   - archived is not live.
 *
 * An empty mode with no ended status IS live, exactly as the twin treats it:
 * that is the desktop-shaped row carrying its state in `status`, and refusing it
 * would revoke roots the brain grants.
 *
 * TWIN: snapshotLive in services/hub/cmd/brain/visibility.go.
 */
export function snapshotGrantsFsRoot(snap: unknown): boolean {
  if (typeof snap !== 'object' || snap === null || Array.isArray(snap)) return false;
  const row = snap as Record<string, unknown>;
  // Read defensively rather than off ClaudeSessionSnapshot: the store also
  // carries claudemon-shaped rows through the enrich overlay, and a field of the
  // wrong TYPE is the "will not decode" clause — it must not read as "no mode,
  // therefore live", which is exactly how a shape change widens an allow-list.
  if (row.mode !== undefined && typeof row.mode !== 'string') return false;
  if (row.status !== undefined && typeof row.status !== 'string') return false;
  if (row.archived !== undefined && typeof row.archived !== 'boolean') return false;
  const mode = (row.mode as string | undefined) ?? '';
  const status = (row.status as string | undefined) ?? '';
  if (mode === 'unknown' || row.archived === true) return false;
  return !(mode === 'stopped' || (mode === '' && status === 'ended'));
}
