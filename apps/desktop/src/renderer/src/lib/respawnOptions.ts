/**
 * The spawn options a respawn derives from a stopped agent's RECORD — pure, so
 * the record → respawn → spawn-IPC round trip is testable. A respawn must
 * re-pass everything the original spawn recorded, or the revived session comes
 * back subtly different: dropping `manager`/`supervisor` here was exactly the
 * regression where a respawned Fleet Manager re-minted its facade token with
 * no grants and every dispatched worker's skipPermissions got clamped.
 */
import type { AgentWorkspace } from '../types/pane';

export interface RespawnSpawnOptions {
  cwd: string;
  provider?: AgentWorkspace['provider'];
  transport?: 'pty' | 'stream';
  profileId?: string;
  model?: string;
  effort?: string;
  permissionMode?: string;
  skipPermissions?: boolean;
  mcpItemIds?: string[];
  toolScope?: 'view' | 'triage' | 'operator';
  pluginTools?: string[];
  supervisor?: boolean;
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
    effort: agent.effort,
    permissionMode: agent.permissionMode,
    skipPermissions: agent.skipPermissions,
    mcpItemIds: agent.mcpItemIds,
    toolScope: agent.toolScope,
    pluginTools: agent.pluginTools,
    // Role flags: the re-minted facade token's grants (profilesAllowed, the
    // config-resolved yolo grant, the role tag) all hang off these.
    supervisor: agent.supervisor,
    manager: agent.manager,
    fleetFullAccess: agent.fleetFullAccess,
    resumeSessionId,
    cols: 120,
    rows: 32,
  };
}
