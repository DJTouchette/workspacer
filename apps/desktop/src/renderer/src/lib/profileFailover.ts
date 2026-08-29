/**
 * Automatic account failover: when a session's Claude account exhausts a usage
 * window, restart the session onto another profile — "cycle through until one
 * works". A profile opts into the rotation with a positive `weight` (Settings
 * → Claude Profiles); heavier wins first. The pane owns the trigger (it sees
 * the statusLine windows) and the restart; this module owns the CHOICE, so the
 * ordering rules are testable without a pane.
 */

import type { ClaudeProfile } from '../../../main/shared/ipcTypes';
import { PROFILE_CAPS, profileProviderOf } from '../../../main/shared/agentProfiles';

/** How long a profile that just hit its limit stays out of the rotation.
 *  Roughly "don't retry an exhausted account until its window had a chance to
 *  move" without modeling reset times — the cycle finds out by trying. */
export const FAILOVER_BLOCK_MS = 10 * 60_000;

/**
 * Pick the profile to fail over to, or null when there's nowhere to go.
 *
 *   - candidates: the SAME HARNESS as the session. Once profiles can be Codex
 *     or Copilot ones, an unfiltered rotation would restart a Claude session
 *     onto a profile whose configDir is a CODEX_HOME — a config root of the
 *     wrong harness entirely. The harness is also what decides whether a
 *     rotation is possible at all (PROFILE_CAPS.failoverWeight): Copilot emits
 *     no usage-window percentage, so nothing could ever trigger one;
 *   - weight > 0 (weights ARE the opt-in setting), heaviest first, name as the
 *     tiebreak so the order is stable;
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
  /** The harness the exhausted session runs. Defaults to Claude, which is what
   *  every caller meant before harnesses existed and what a profile with no
   *  `provider` key is. */
  provider: string = 'claude',
): ClaudeProfile | null {
  if (!(provider in PROFILE_CAPS)) return null;
  const caps = PROFILE_CAPS[provider as keyof typeof PROFILE_CAPS];
  if (!caps.failoverWeight) return null;
  const sameHarness = profiles.filter((p) => profileProviderOf(p) === provider);
  const currentId = currentProfileId ?? sameHarness.find((p) => p.isDefault)?.id;
  const candidates = sameHarness
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

/**
 * Whether an automatic rotation is possible at all for this harness — the gate
 * the pane's trigger reads, so "which harnesses rotate" is answered in ONE
 * place and it is the capability table.
 *
 * False for a harness with no profiles (OpenCode, Pi) and for one that reports
 * no usage-window percentage to rotate off (Copilot). True for Claude and for
 * Codex, whose primary/secondary windows claudemon maps onto the same
 * five_hour_pct / seven_day_pct `windowExhausted` reads.
 */
export function profileFailoverPossible(provider: string | undefined): boolean {
  if (!provider || !(provider in PROFILE_CAPS)) return false;
  return PROFILE_CAPS[provider as keyof typeof PROFILE_CAPS].failoverWeight;
}

/** The trigger: a usage window effectively exhausted. 99.5 rather than 100 —
 *  the endpoint rounds, and a capped account can report either. */
export function windowExhausted(fiveHourPct?: number, sevenDayPct?: number): boolean {
  return (fiveHourPct ?? 0) >= 99.5 || (sevenDayPct ?? 0) >= 99.5;
}
