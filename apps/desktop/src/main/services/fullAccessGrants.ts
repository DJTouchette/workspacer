/**
 * Full-access (yolo) grant policy for role-tagged session facade tokens.
 *
 * The grant lives on the per-session token record (tokens.json), but the
 * POLICY lives in config: agents.fleetFullAccess / per-project `yolo` for the
 * Fleet Manager, supervisor.fullAccess for supervisors. This module is the one
 * place that formula is spelled, shared by
 *   - the mint paths (claudeSpawn / managedSpawn) — a fresh or RESPAWNED
 *     manager/supervisor mints with the grant config says it should have, so a
 *     flag flipped while the session was stopped never resurrects (or
 *     withholds) a grant via a stale caller flag, and
 *   - the live reconciler below — the MCP facade re-reads the token record per
 *     request, so rewriting yoloAllowed when the flag flips applies the change
 *     to RUNNING managers/supervisors immediately, in both directions
 *     (granting and revoking), no respawn needed.
 */
import { agentNotifier } from './agentNotifier';
import { configService } from './configService';
import {
  reconcileSessionFacadeGrants,
  type SessionGrantFlip,
  type SessionTokenRole,
} from './remoteTokens';

/**
 * Whether the Fleet Manager's token should carry the full-access grant: global
 * agents.fleetFullAccess, OR any project opting into per-project yolo — else
 * the manager's per-project bypassed dispatches (doctrine rule 7) would be
 * clamped for want of the grant. TWIN: the renderer computes the same formula
 * for the manager's own bypass note (App.tsx fleet-manager entry).
 */
export function managerFullAccessFromConfig(): boolean {
  const cfg = configService.getConfig();
  if (cfg.agents?.fleetFullAccess === true) return true;
  return Object.values(cfg.projects ?? {}).some((p) => p?.yolo === true);
}

/** Whether a supervisor's token should carry the full-access grant. */
export function supervisorFullAccessFromConfig(): boolean {
  return configService.getConfig().supervisor?.fullAccess === true;
}

/** The grant each session role should hold under current config. */
export function desiredSessionGrants(): Record<SessionTokenRole, boolean> {
  return {
    manager: managerFullAccessFromConfig(),
    supervisor: supervisorFullAccessFromConfig(),
  };
}

/**
 * One-shot reconcile of every role-tagged session token against current
 * config. Returns how many token records changed.
 *
 * `announce` posts a notification for each live session whose grant actually
 * moved — see announceGrantFlips for why. Off for the boot reconcile: catching
 * up a flag flipped while the desktop was closed is not news.
 */
export function reconcileFullAccessGrants(announce = false): number {
  const flips = reconcileSessionFacadeGrants(desiredSessionGrants());
  if (flips.length) {
    console.log(
      `[fullAccessGrants] reconciled ${flips.length} session token grant(s) with config full-access flags`,
    );
    if (announce) announceGrantFlips(flips);
  }
  return flips.length;
}

const ROLE_LABEL: Record<SessionTokenRole, string> = {
  manager: 'Fleet Manager',
  supervisor: 'Supervisor',
};

/**
 * Say out loud what a full-access flip did and — just as importantly — what it
 * did NOT do.
 *
 * Half of this setting applies live and half cannot, and silently doing nothing
 * visible is what made the whole thing undiagnosable. What IS live: the facade
 * re-reads the token record per request, so the agents a running manager
 * dispatches from now on are judged under the new grant immediately (and with
 * the grant now ADDING the bypass, that is the whole fix). What can NEVER be
 * live: a session's own permission bypass is fixed at spawn and minted into its
 * process, so the manager's own tool calls keep whatever mode it was started
 * with until it is respawned.
 *
 * Deliberately a NOTIFICATION, not an automatic respawn. Respawning a live
 * manager mid-fleet would drop the conversation that knows what every worker
 * was dispatched to do — a far more expensive surprise than a prompt. The
 * notification carries the manager's sessionId, so clicking it selects that
 * agent and the existing respawn control on its card is one click away: the
 * respawn is OFFERED, and stays the user's decision.
 *
 * Only records that actually changed are announced, so toggling the flag with
 * no manager running says nothing.
 */
function announceGrantFlips(flips: SessionGrantFlip[]): void {
  for (const flip of flips) {
    const who = ROLE_LABEL[flip.role];
    agentNotifier.postInApp({
      // Keyed per session+role: flipping back and forth replaces the earlier
      // note instead of stacking a pile of contradictory ones.
      key: `full-access:${flip.role}:${flip.sessionId}`,
      level: 'info',
      source: 'system',
      sessionId: flip.sessionId,
      title: flip.yoloAllowed ? `Full access on for the ${who}` : `Full access off for the ${who}`,
      body: flip.yoloAllowed
        ? `Agents it dispatches from now on skip approval prompts. Its own tool calls do not — a session's bypass is fixed when it spawns. Open it and respawn if you want the ${who} itself running with full access.`
        : `Agents it dispatches from now on ask for approval again. Workers already running keep the mode they started with, and so does the ${who}'s own session until it is respawned.`,
    });
  }
}

let started = false;

/**
 * Start watching config for full-access flag flips and keep live session
 * tokens' grants in line. Config changes from ANY writer count — Settings,
 * main's own saves, the brain, a hand edit (the config watcher catches those).
 * Call once at boot; also run reconcileFullAccessGrants() there to catch flips
 * that happened while the desktop wasn't running.
 */
export function startFullAccessGrantSync(): void {
  if (started) return;
  started = true;
  configService.onChange(() => {
    try {
      // announce: a flip from here is something the user just did (or a writer
      // did on their behalf), so the half that cannot apply live must be said.
      reconcileFullAccessGrants(true);
    } catch (err) {
      console.error('[fullAccessGrants] grant reconcile failed:', err);
    }
  });
}
