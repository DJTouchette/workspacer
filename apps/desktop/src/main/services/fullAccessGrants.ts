/**
 * Compatibility shims for the retired session-grant reconciler.
 *
 * Supported agents now receive the ambient Workspacer operator facade and
 * Workspacer does not use config flags or token-record yolo fields to grant or
 * revoke a child's provider permission mode. Legacy config and token fields
 * remain parseable so upgrades are lossless, but they are never consulted or
 * rewritten here.
 */
import type { SessionTokenRole } from './remoteTokens';

/** @deprecated Legacy compatibility only; no token policy is derived from config. */
export function managerFullAccessFromConfig(): boolean {
  return false;
}

/** @deprecated Legacy compatibility only; values are inert. */
export function desiredSessionGrants(): Record<SessionTokenRole, boolean> {
  return { manager: false };
}

/** @deprecated Grant reconciliation was removed; no records are mutated. */
export function reconcileFullAccessGrants(_announce = false): number {
  return 0;
}

/** @deprecated Grant reconciliation was removed; this intentionally installs no watcher. */
export function startFullAccessGrantSync(): void {}
