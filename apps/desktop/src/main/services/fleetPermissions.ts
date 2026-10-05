import { configService } from './configService';
import { claudeSessionStore } from './claudeSessionStore';

/** User-selected provider approval policy, never a facade/token grant.
 * Read live config for each launch so a running manager's next dispatch sees
 * settings changes. Follow recorded lineage only, with a cycle guard.
 */
export function fleetSkipsPermissions(
  opts: { manager?: boolean; parentSessionId?: string },
  enabled = configService.getConfig().agents?.fleetFullAccess === true,
): boolean {
  if (!enabled) return false;
  if (opts.manager) return true;
  const seen = new Set<string>();
  let parentId = opts.parentSessionId;
  while (parentId && !seen.has(parentId)) {
    seen.add(parentId);
    const parent = claudeSessionStore.getSnapshot(parentId);
    if (!parent) return false;
    if (parent.isWakeTarget || parent.isFleetManager) return true;
    parentId = parent.parentSessionId;
  }
  return false;
}

/** `agents.childFullAccess`: the user's explicit choice that NEW child
 * launches of this host's own sessions bypass provider approvals (Claude
 * bypassPermissions, Codex full access). Provider policy only, never a
 * Workspacer approval gate or facade/token grant. Parentless, unknown- or
 * foreign-parent and resumed launches keep their own mode, as do running
 * sessions. Local launchers only: paired/remote dispatch never takes it.
 * Mirrors the Rust hub's spawn_plan so both launchers agree.
 */
export function childSkipsPermissions(
  opts: { manager?: boolean; parentSessionId?: string; resumeSessionId?: string },
  enabled = configService.getConfig().agents?.childFullAccess === true,
): boolean {
  if (!enabled || opts.manager || opts.resumeSessionId || !opts.parentSessionId) return false;
  const parent = claudeSessionStore.getSnapshot(opts.parentSessionId);
  return !!parent && !parent.hub;
}
