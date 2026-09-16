/**
 * The spawn options a respawn derives from a stopped agent's RECORD — pure, so
 * the record → respawn → spawn-IPC round trip is testable. A respawn must
 * re-pass everything the original spawn recorded, or the revived session comes
 * back subtly different: dropping `manager` here was exactly the regression
 * where a respawned Fleet Manager lost its role metadata and
 * every dispatched worker's skipPermissions got clamped.
 */
import type { AgentWorkspace } from '../types/pane';

export interface RespawnSpawnOptions {
  cwd: string;
  provider?: AgentWorkspace['provider'];
  transport?: 'pty' | 'stream';
  profileId?: string;
  launchIntegrationId?: string | null;
  model?: string;
  modelIdentity?: string;
  contextWindow?: number | null;
  effort?: string;
  permissionMode?: string;
  skipPermissions?: boolean;
  mcpItemIds?: string[];
  toolScope?: 'view' | 'triage' | 'operator';
  pluginTools?: string[];
  manager?: boolean;
  fleetFullAccess?: boolean;
  resumeSessionId?: string;
  cols: number;
  rows: number;
}

export function buildRespawnSpawnOptions(
  agent: AgentWorkspace,
  resumeSessionId: string | undefined,
): RespawnSpawnOptions {
  // Claude respawns follow the config default transport unless the agent
  // explicitly ran stream — a recorded 'pty' is usually just the legacy
  // default, and users who flipped their default to stream expect old
  // chats to come back in it. Managed providers keep their transport
  // (codex 'pty' is the native TUI, a genuinely different frontend).
  const transport =
    agent.provider && agent.provider !== 'claude'
      ? agent.transport
      : agent.transport === 'stream'
        ? ('stream' as const)
        : undefined;
  return {
    cwd: agent.cwd,
    provider: agent.provider,
    transport,
    profileId: agent.profileId,
    launchIntegrationId: agent.launchIntegrationId,
    model: agent.model,
    modelIdentity: agent.modelIdentity,
    contextWindow: agent.contextWindow,
    effort: agent.effort,
    permissionMode: agent.permissionMode,
    skipPermissions: agent.skipPermissions,
    mcpItemIds: agent.mcpItemIds,
    // Role flag: keeps manager wake routing and role metadata across respawn.
    manager: agent.manager,
    fleetFullAccess: agent.fleetFullAccess,
    resumeSessionId,
    cols: 120,
    rows: 32,
  };
}
