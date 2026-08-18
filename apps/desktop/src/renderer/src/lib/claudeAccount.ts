/**
 * Claude account identity for LOCAL sessions, derived from the transcript
 * path Claude Code writes under its config dir (`<root>/projects/<slug>/…`).
 *
 * Two logins are two `CLAUDE_CONFIG_DIR`s ("Add Claude Account" profiles put
 * them at `~/.claude/accounts/<name>`), and their rate-limit windows are
 * unrelated — so anything account-scoped in the UI must group sessions by
 * this key instead of treating the freshest reading as global. Remote
 * (federated) sessions have their transcript path blanked by the bridge and
 * collapse to the default group; their gauges are per-session anyway.
 */

/** Group label for a session: '' = the default login, otherwise the account's
 *  short name (`accounts/<slug>` → slug, a hand-made config dir → its
 *  basename). Derived, so it needs no wire or snapshot-shape changes. */
export function claudeAccountOf(transcriptPath: string | undefined): string {
  if (!transcriptPath) return '';
  const norm = transcriptPath.replace(/\\/g, '/');
  // The slug after /projects/ is a single dash-flattened component, so the
  // LAST occurrence bounds the config root even if the root's own path
  // contains a projects directory.
  const idx = norm.lastIndexOf('/projects/');
  if (idx <= 0) return '';
  const root = norm.slice(0, idx).split('/').filter(Boolean);
  const base = root[root.length - 1] ?? '';
  if (root[root.length - 2] === 'accounts') return base;
  return base === '.claude' ? '' : base;
}
