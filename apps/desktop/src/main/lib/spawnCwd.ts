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
 *
 * The TRIM SET is spelled out rather than delegated to `String.trim`. JS
 * `.trim()` and Go's `strings.TrimSpace` are not the same function, and the two
 * differences point in opposite directions:
 *
 *   U+FEFF (ZWNBSP/BOM)  in ECMAScript's WhiteSpace production, NOT in Go's
 *                        `unicode.IsSpace` — so `{"cwd":"<U+FEFF>"}` trimmed to
 *                        empty here and became `$HOME`, while the brain ran the
 *                        agent in a directory literally named U+FEFF.
 *   U+0085 (NEL)         `unicode.IsSpace` in Go, neither <USP> nor a
 *                        LineTerminator in JS — the same split, reversed.
 *
 * A BOM is exactly what a path pasted out of a Windows editor carries, and this
 * string is what lands in `workspaceRoots()`, so the disagreement is "$HOME is
 * an fs.* root" versus "a nonexistent directory is". Neither built-in is
 * portable, so both copies trim the ASCII whitespace set and nothing else; every
 * other code point is an ordinary character in a filename.
 */
import * as os from 'os';

/** The whitespace `normalizeSpawnCwd` strips — space, tab and the four ASCII
 *  vertical/form controls. The Go twin carries the identical literal set. */
const TRIM_SET = /^[ \t\n\v\f\r]+|[ \t\n\v\f\r]+$/g;

export function normalizeSpawnCwd(cwd: string | undefined | null): string {
  let s = (cwd ?? '').replace(TRIM_SET, '');
  while (s.length > 1 && (s.endsWith('/') || s.endsWith('\\'))) s = s.slice(0, -1);
  return s === '' ? os.homedir() : s;
}
