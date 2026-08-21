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
import { configService } from './configService';
import { reconcileSessionFacadeGrants, type SessionTokenRole } from './remoteTokens';

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
 */
export function reconcileFullAccessGrants(): number {
  const n = reconcileSessionFacadeGrants(desiredSessionGrants());
  if (n) {
    console.log(
      `[fullAccessGrants] reconciled ${n} session token grant(s) with config full-access flags`,
    );
  }
  return n;
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
      reconcileFullAccessGrants();
    } catch (err) {
      console.error('[fullAccessGrants] grant reconcile failed:', err);
    }
  });
}
