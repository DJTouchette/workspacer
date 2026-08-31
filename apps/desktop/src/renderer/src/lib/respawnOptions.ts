/**
 * The spawn options a respawn derives from a stopped agent's RECORD — pure, so
 * the record → respawn → spawn-IPC round trip is testable. A respawn must
 * re-pass everything the original spawn recorded, or the revived session comes
 * back subtly different: dropping `manager` here was exactly the regression
 * where a respawned Fleet Manager re-minted its facade token with no grants and
 * every dispatched worker's skipPermissions got clamped.
 */
import type { AgentWorkspace } from '../types/pane';

export interface RespawnSpawnOptions {
  cwd: string;
  provider?: AgentWorkspace['provider'];
  transport?: 'pty' | 'stream';
  profileId?: string;
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
    model: agent.model,
    modelIdentity: agent.modelIdentity,
    contextWindow: agent.contextWindow,
    effort: agent.effort,
    permissionMode: agent.permissionMode,
    skipPermissions: agent.skipPermissions,
    mcpItemIds: agent.mcpItemIds,
    // A card saved as a supervisor (the retired fleet role) has no toolScope of
    // its own — the role implied 'operator'. Heal it here so respawning an old
    // supervisor card revives an operator-tier facade agent instead of one with
    // no workspacer tools at all.
    toolScope: agent.toolScope ?? (legacySupervisorRecord(agent) ? 'operator' : undefined),
    pluginTools: agent.pluginTools,
    // Role flag: the re-minted facade token's grants (profilesAllowed, the
    // config-resolved yolo grant, the role tag) all hang off it.
    manager: agent.manager,
    fleetFullAccess: agent.fleetFullAccess,
    resumeSessionId,
    cols: 120,
    rows: 32,
  };
}

/** A card persisted before the supervisor role was removed: the flags are gone
 *  from AgentWorkspace, but the saved JSON still carries them. */
function legacySupervisorRecord(agent: AgentWorkspace): boolean {
  const legacy = agent as AgentWorkspace & { supervisor?: boolean; kind?: string };
  return legacy.supervisor === true || legacy.kind === 'supervisor';
}
