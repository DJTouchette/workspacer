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
