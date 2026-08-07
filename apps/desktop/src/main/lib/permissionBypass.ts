/**
 * The one place that answers "does this permission mode turn the host's
 * approvals OFF?" — for every bus path that accepts a mode from a caller who is
 * not the local user.
 *
 * `agents.spawn` has always carried the invariant in prose: a remote/web/MCP
 * caller may start an agent, but never one that silently auto-bypasses every
 * approval — "a YOLO agent must be started locally". It enforced that with two
 * inline string comparisons (`'bypassPermissions'`, `'yolo'`) inside the spawn
 * handler, which made the invariant look like a property of spawning. It is not.
 * It is a property of the MODE, and the mode arrives through more than one door:
 *
 *   agents.spawn({ permissionMode: 'yolo' })         → clamped since day one
 *   claude.setPermissionMode({ sessionId, mode })    → was not clamped at all
 *
 * The second reaches an agent that is ALREADY RUNNING — including one the local
 * user started in ask mode, since neither provider ownership-checks the
 * sessionId — and claudemon applies it for real (PTY claude by cycling Shift+Tab
 * to the bypass footer, managed providers by flipping the adapter's auto-approve
 * flag). One extra bus call therefore undid the spawn clamp entirely, and then
 * `agents.sendMessage` drove the now-unsupervised agent.
 *
 * ALLOWLIST, not denylist. The modes in flight across the four providers are
 * `default`/`acceptEdits`/`plan`/`bypassPermissions` (claude, both transports),
 * `ask`/`yolo` (codex, opencode, pi), and `auto`/`dontAsk`/`manual`, which
 * claudemon's stream endpoint also accepts and which appear in live telemetry
 * without being offered in any menu. A denylist has to name every spelling of
 * "stop asking" that any provider will ever ship; an allowlist names the ones we
 * have checked mean "keep asking (or ask MORE)" and fails closed on everything
 * else — the same reasoning that made terminals.create's `shell` an allowlist of
 * login shells rather than a list of forbidden binaries.
 */

/**
 * Permission modes a bus caller may request: the neutral one and the ones that
 * only ever ADD friction. Ordered as the pills present them.
 *
 *  - `default` / `ask` — the provider's ordinary "ask before each tool call".
 *  - `plan` — read-only planning; strictly more restrictive than default.
 *  - `manual` — the stream endpoint's spelling of "I approve each step".
 *
 * `acceptEdits` is deliberately IN: it is what the desktop's own pill offers as
 * a normal working mode, `agents.spawn` passes it through for a bus caller
 * today, and it still gates every non-edit tool call (Bash included). Removing
 * it here would make the live switch refuse a mode the spawn path grants, which
 * is a difference this module exists to eliminate rather than create.
 */
export const BUS_SETTABLE_PERMISSION_MODES: ReadonlySet<string> = new Set([
  'default',
  'ask',
  'acceptEdits',
  'plan',
  'manual',
]);

/**
 * True when `mode` is a value a bus caller must not be able to put a session
 * into: `bypassPermissions`, `yolo`, `dontAsk`, `auto`, and anything a provider
 * invents next. An empty/absent mode is NOT an escalation — the caller is asking
 * for the default — so callers that treat "" as "unset" keep working.
 */
export function isPermissionEscalation(mode: string | undefined | null): boolean {
  if (!mode) return false;
  return !BUS_SETTABLE_PERMISSION_MODES.has(mode);
}

/**
 * Throw unless `mode` is one a bus caller may set on a live session. Returns the
 * mode so the call site can use the CHECKED value rather than re-reading the
 * caller's variable — the same check-path/used-path rule assertPathAllowed
 * follows.
 *
 * The message names the mode: unlike a path refusal (where confirming where a
 * denied target landed is a probe primitive) the mode is a fixed, public
 * vocabulary the caller already knows, and a remote pill needs to be able to say
 * why the switch did not take.
 */
export function assertNoPermissionBypass(cap: string, mode: string): string {
  if (isPermissionEscalation(mode)) {
    throw new Error(
      `${cap}: a bus caller cannot switch a running session into '${mode}' — ` +
        'auto-approving modes must be chosen locally (the same rule agents.spawn applies at spawn time)',
    );
  }
  return mode;
}
