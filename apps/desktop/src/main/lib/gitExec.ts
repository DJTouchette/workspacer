/**
 * The argument prefix every `git` invocation in this process must carry.
 *
 * `.git/config` is data, and on the bus surface it is CALLER-WRITTEN data. Every
 * directory these commands run in is either an agent cwd or a config store, and
 * both are writable through `fs.write` — `<configDir>/library` is a
 * configStoreRoot, and writeHostFile creates missing parents, so a bus client
 * with nothing but fs.write can mint `<configDir>/library/.git/config` without a
 * repository existing first. git then discovers that repository at the cwd,
 * reads `core.fsmonitor` out of the caller's file and EXECUTES it: a shell
 * command as the desktop user, with no agent approval and no plugin sandbox,
 * from a read-only directory listing. The path guard cannot help — the entire
 * chain lives inside an allowed root.
 *
 * `-c` on the command line outranks every config file, which is why this is a
 * prefix rather than an environment variable. It must be spread BEFORE the
 * subcommand.
 *
 * THE ANSWER IS PER SUBCOMMAND, which is why this is a list and not one key.
 * `core.fsmonitor` is the only one that fires during `check-ignore`, but the same
 * prefix is shared with the git.*, worktree and replay runners, and there
 * `diff.external` fires on `git diff` and `filter.<drv>.clean` fires on `git add`
 * — both measured, both reached from a plain 0644 file a caller can write.
 *
 * Three mechanisms, because no one of them covers the surface:
 *
 *  1. The `-c` list below: every exec-valued key with a FIXED name. An empty
 *     value is the documented "no program" spelling for each of them, and
 *     `credential.helper=` additionally RESETS the inherited helper list.
 *     `diff.external` is deliberately NOT here — git treats an empty external
 *     diff as a command to run and dies with "cannot run :", so neutralizing it
 *     that way breaks every real diff. `gpg.program` is not here either: the
 *     user's own `commit.gpgsign` is legitimate and an empty value would break
 *     signed commits.
 *  2. `--no-ext-diff`, inserted after the subcommand for the diff family by
 *     `gitArgs` itself. That is git's own documented off switch for
 *     `diff.external` (and it is what covers `gpg.program`'s sibling risk too,
 *     since no diff driver runs).
 *  3. The shared path guard. What 1 and 2 structurally cannot cover is the
 *     NAMESPACED exec keys — `filter.<drv>.clean/smudge/process`,
 *     `diff.<drv>.command/textconv`, `merge.<drv>.driver`, `trailer.<t>.command`
 *     — because the driver name is the attacker's, so no fixed list can name
 *     them. Every one of those drivers has to be DEFINED in a config file inside
 *     the repository's `.git` directory, and pathConfinement.ts `isSecretPath`
 *     (with its Go twins) refuses every caller-supplied path that traverses a
 *     `.git` component. The definition can never be written.
 *
 * GIT_CONFIG_GLOBAL is deliberately NOT neutralized: `core.excludesFile` in the
 * user's own ~/.gitconfig is a legitimate part of the ignore answer, and the
 * user's home config is not the attacker's. Mirrors gitNoExecConfig() in
 * services/hub/cmd/brain/fsops.go.
 */
export const GIT_NO_EXEC_KEYS: readonly string[] = [
  'core.fsmonitor=',
  'core.pager=cat',
  'core.sshCommand=',
  'core.askPass=',
  'core.editor=',
  'core.alternateRefsCommand=',
  'core.gitProxy=',
  'credential.helper=',
  'sequence.editor=',
  'uploadpack.packObjectsHook=',
];

export const GIT_NO_EXEC_CONFIG: readonly string[] = GIT_NO_EXEC_KEYS.flatMap((kv) => ['-c', kv]);

/**
 * Subcommands that generate a diff and therefore honour `diff.external` and the
 * per-driver `textconv`/`command` drivers. `--no-ext-diff` is a diff OPTION, not
 * a global one, so it cannot live in the `-c` prefix — it has to be inserted
 * after the subcommand, which is why `gitArgs` does it rather than every caller.
 */
const DIFF_FAMILY = new Set([
  'diff',
  'show',
  'log',
  'format-patch',
  'diff-index',
  'diff-tree',
  'diff-files',
  'range-diff',
  'whatchanged',
]);

/** `git` argv with the no-exec prefix in front of the subcommand, and
 *  `--no-ext-diff` immediately after it when the subcommand generates a diff.
 *
 *  The subcommand is the first element that is not a `-c` pair, because callers
 *  are allowed to pass their own leading `-c` (git.numstat passes
 *  `core.quotepath=false`). */
export function gitArgs(args: readonly string[]): string[] {
  const out = [...GIT_NO_EXEC_CONFIG, ...args];
  for (let i = GIT_NO_EXEC_CONFIG.length; i < out.length; i++) {
    if (out[i] === '-c') {
      i += 1; // skip the key=value that follows
      continue;
    }
    if (DIFF_FAMILY.has(out[i])) out.splice(i + 1, 0, '--no-ext-diff');
    break;
  }
  return out;
}
