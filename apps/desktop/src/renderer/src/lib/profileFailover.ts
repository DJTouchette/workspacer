/**
 * Automatic account failover: when a session's Claude account exhausts a usage
 * window, restart the session onto another profile — "cycle through until one
 * works". A profile opts into the rotation with a positive `weight` (Settings
 * → Claude Profiles); heavier wins first. The pane owns the trigger (it sees
 * the statusLine windows) and the restart; this module owns the CHOICE, so the
 * ordering rules are testable without a pane.
 */

import type { ClaudeProfile } from '../../../main/shared/ipcTypes';

/** How long a profile that just hit its limit stays out of the rotation.
 *  Roughly "don't retry an exhausted account until its window had a chance to
 *  move" without modeling reset times — the cycle finds out by trying. */
export const FAILOVER_BLOCK_MS = 10 * 60_000;

/**
 * Pick the profile to fail over to, or null when there's nowhere to go.
 *
 *   - candidates: weight > 0 (weights ARE the opt-in setting), heaviest first,
 *     name as the tiebreak so the order is stable;
 *   - never the profile the session is already on;
 *   - never a profile whose account has no login yet (signedIn === false —
 *     an UNKNOWN status keeps the candidate: the restart will show the
 *     sign-in banner rather than silently doing nothing);
 *   - never one that hit its own limit within FAILOVER_BLOCK_MS (that's the
 *     "until one works" part — each failure blocks that stop on the cycle).
 *
 * `currentProfileId` may be undefined (the pane spawned with no profile):
 * that means the DEFAULT profile, so the default row is excluded then too.
 */
export function pickFailoverProfile(
  profiles: ClaudeProfile[],
  currentProfileId: string | undefined,
  signedIn: Record<string, boolean>,
  blockedAt: Map<string, number>,
  now: number,
): ClaudeProfile | null {
  const currentId = currentProfileId ?? profiles.find((p) => p.isDefault)?.id;
  const candidates = profiles
    .filter((p) => (p.weight ?? 0) > 0)
    .filter((p) => p.id !== currentId)
    .filter((p) => signedIn[p.id] !== false)
    .filter((p) => {
      const at = blockedAt.get(p.id);
      return !at || now - at > FAILOVER_BLOCK_MS;
    })
    .sort((a, b) => (b.weight ?? 0) - (a.weight ?? 0) || a.name.localeCompare(b.name));
  return candidates[0] ?? null;
}

/** The trigger: a usage window effectively exhausted. 99.5 rather than 100 —
 *  the endpoint rounds, and a capped account can report either. */
export function windowExhausted(fiveHourPct?: number, sevenDayPct?: number): boolean {
  return (fiveHourPct ?? 0) >= 99.5 || (sevenDayPct ?? 0) >= 99.5;
}
