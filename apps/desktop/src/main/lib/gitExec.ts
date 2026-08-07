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
 * `core.fsmonitor` is the only key that fires during `check-ignore`
 * (core.pager, diff.external, core.sshCommand, core.alternateRefsCommand and
 * credential.helper were all probed and do not), but this stays a list because
 * the answer is per-subcommand and the same prefix is shared with the git.*,
 * worktree and replay runners.
 *
 * GIT_CONFIG_GLOBAL is deliberately NOT neutralized: `core.excludesFile` in the
 * user's own ~/.gitconfig is a legitimate part of the ignore answer, and the
 * user's home config is not the attacker's. Mirrors gitNoExecConfig() in
 * services/hub/cmd/brain/fsops.go.
 */
export const GIT_NO_EXEC_CONFIG: readonly string[] = ['-c', 'core.fsmonitor='];

/** `git` argv with the no-exec prefix in front of the subcommand. */
export function gitArgs(args: readonly string[]): string[] {
  return [...GIT_NO_EXEC_CONFIG, ...args];
}
