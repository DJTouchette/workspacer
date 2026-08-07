/**
 * The `shell` param of `terminals.create` is argv[0] of a process spawned on the
 * host, taken verbatim from a bus caller.
 *
 * capspec leaves `terminals.create` unscoped and its recorded reason named ONE
 * param: "cwd is a process working directory, and holding the capability at all
 * is the gate". That was incomplete in exactly the way `sessions.*`'s silence
 * was — `shell` is a second, unnamed process identifier, it was not in the params
 * scanner's path-ish set, and neither provider checked it at all. Combined with a
 * mode-preserving `fs.write` over an existing executable inside the caller's own
 * agent cwd (`writeFileSync` keeps the 0755), `terminals.create` alone was
 * arbitrary host code execution.
 *
 * An ALLOWLIST rather than containment, because there is no subtree we could
 * confine this to that the same caller cannot also fill in — the same argument
 * `scrubBypassProfile` makes about CLAUDE_CONFIG_DIR. A shell is one of the login
 * shells the host already trusts: the user's `$SHELL`, what `/etc/shells` lists,
 * and the platform fallbacks.
 *
 * TWIN: services/hub/cmd/brain/shellallow.go.
 */
import fs from 'fs';
import path from 'path';

/** Always-allowed platform defaults — what terminals.create already used when
 *  the caller named nothing, so refusing them would refuse the ordinary call. */
function fallbackShells(): string[] {
  if (process.platform === 'win32') return ['powershell.exe', 'pwsh.exe', 'cmd.exe'];
  return [
    '/bin/sh',
    '/bin/bash',
    '/bin/zsh',
    '/usr/bin/bash',
    '/usr/bin/zsh',
    '/bin/fish',
    '/usr/bin/fish',
  ];
}

/** Overridable so a test can point at a fixture instead of the real /etc/shells. */
export const shellConfig = { etcShellsPath: '/etc/shells' };

/** $SHELL, /etc/shells and the platform fallbacks. Read at call time — installing
 *  a new shell should not require restarting the app. */
function allowedShells(): Set<string> {
  const set = new Set<string>();
  const add = (s: string | undefined): void => {
    const v = (s ?? '').trim();
    if (!v || v.startsWith('#')) return;
    set.add(v);
  };
  for (const s of fallbackShells()) add(s);
  add(process.env.SHELL);
  try {
    for (const line of fs.readFileSync(shellConfig.etcShellsPath, 'utf-8').split('\n')) add(line);
  } catch {
    // No /etc/shells (Windows, a locked-down container): the fallbacks stand.
  }
  return set;
}

/** The host's own default shell — what an empty request gets. */
export function defaultShell(): string {
  return process.env.SHELL || fallbackShells()[0];
}

/**
 * Resolve a caller-supplied `shell` to the argv[0] a terminal may actually be
 * spawned with, or `null` to refuse the call.
 *
 * Refusing is deliberate: silently substituting the default would hide the
 * denial from the caller and make the allowlist untestable from outside.
 */
export function resolveTerminalShell(requested: string | undefined): string | null {
  if (!requested || !requested.trim()) return defaultShell();
  const allowed = allowedShells();
  if (allowed.has(requested)) return requested;
  if (process.platform === 'win32') {
    // A bare command name is the ordinary Windows spelling; compare on basename.
    const base = path.basename(requested).toLowerCase();
    for (const s of allowed) if (path.basename(s).toLowerCase() === base) return requested;
  }
  return null;
}
