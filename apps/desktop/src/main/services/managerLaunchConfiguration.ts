import { createHash } from 'crypto';
import { claudeProfiles } from './claudeProfiles';
import type { ManagedSpawnOptions } from './managedSpawn';

/** Configuration outside session telemetry must remain unchanged. A hash
 * proves that without retaining profile credentials in the operation journal. */
export function managerLaunchConfiguration(options: ManagedSpawnOptions): string {
  return createHash('sha256')
    .update(
      JSON.stringify({
        profile: options.profileId ? claudeProfiles.getProfile(options.profileId) : null,
        launchIntegrationId: options.launchIntegrationId ?? null,
      }),
    )
    .digest('hex');
}
