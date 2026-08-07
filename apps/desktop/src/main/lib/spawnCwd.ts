/**
 * The ONE normalization a caller-supplied spawn / terminal working directory
 * gets, shared by both providers of `agents.spawn` and `terminals.create`.
 *
 * `cwd` is not a path the guard confines — capspec lists both methods under
 * `unscopedByDecision` because a process working directory is the point of the
 * call. But the two providers still have to AGREE about what string they hand
 * the daemon, and they did not:
 *
 *   - The brain tilde-expanded it and this side did not, so `{"cwd":"~"}` became
 *     `$HOME` on one provider and the literal `"~"` on the other. That is not
 *     cosmetic: the stored session cwd is what feeds `agentCwds()` into
 *     `workspaceRoots()`, so one caller string turned the whole home tree into an
 *     `fs.*` root on one side and into nothing on the other. BINDING DECISION 1
 *     (pathConfinement.ts's header) says no layer handling caller input expands
 *     '~'; this is such a layer.
 *   - `terminals.create` here did `fs.existsSync(cwd) ? cwd : os.homedir()`,
 *     which silently rewrote the caller's target to somewhere else entirely,
 *     while the brain ran it as given. Five of eight probe spellings opened a
 *     shell in a different directory depending on who answered.
 *
 * So: trim, strip trailing separators, and nothing else. An empty result falls
 * back to the home directory because a terminal has to open SOMEWHERE; a
 * non-empty one is used exactly as written, and a spawn that cannot run where it
 * was asked to fails where it was asked to.
 *
 * The trailing-slash strip is load-bearing rather than tidy: claudemon aliases a
 * spawn to Claude's own session by exact cwd match, and Claude reports its cwd
 * without a trailing slash.
 *
 * TWIN: normalizeCwd in services/hub/cmd/brain/profiles.go. The `spawnCwds`
 * block of contracts/path-containment-cases.json holds the two together.
 */
import * as os from 'os';

export function normalizeSpawnCwd(cwd: string | undefined | null): string {
  let s = (cwd ?? '').trim();
  while (s.length > 1 && (s.endsWith('/') || s.endsWith('\\'))) s = s.slice(0, -1);
  return s === '' ? os.homedir() : s;
}
