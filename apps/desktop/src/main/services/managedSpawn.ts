/**
 * Shared managed-provider (Tier-2) spawn dispatch.
 *
 * OpenCode / Codex / Pi are driven by claudemon's adapters (their own machine
 * interface — `opencode serve`, `codex app-server`, …) rather than a PTY, so
 * spawning one is `POST /sessions/spawn-managed`, not the Claude `argv` spawn.
 *
 * This helper exists so the two entry points that start an agent — the desktop
 * `claude:spawn` IPC handler and the `agents.spawn` hub-bus capability (web /
 * remote client + MCP facade) — share ONE dispatch and can't drift: a provider
 * added or rewired here lights up both transports at once. The bug it fixes:
 * `agents.spawn` used to ignore `provider` and always spawn Claude, so a Codex
 * agent started from the web client came up as a Claude PTY.
 */
import * as os from 'os';
import { randomUUID } from 'crypto';
import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { resolveAgentBinary, type AgentProvider } from './agentProviders';
import { MCP_FACADE_URL, managedFacadeInstructions } from './mcpConfig';
import { ensureSupervisorHome } from './supervisorSkill';

export interface ManagedSpawnOptions {
  /** The managed backend to launch (never 'claude' — caller dispatches that). */
  provider: Exclude<AgentProvider, 'claude'>;
  cwd?: string;
  model?: string;
  /** YOLO / auto-approve every command and file change. */
  skipPermissions?: boolean;
  /** Re-use this id (matches the desktop's pinned-session contract). */
  resumeSessionId?: string;
  /** Wire the workspacer MCP facade + run the /supervise loop. */
  supervisor?: boolean;
  /** Wire the facade tools without the supervisor loop. */
  mcpFacade?: boolean;
  label?: string;
  parentSessionId?: string;
}

/**
 * Spawn a managed (adapter-driven) agent session and return its session id.
 * Mirrors the Claude path's pre-registration: a pinned id + spawn metadata
 * (provider/label/parent) recorded before the first conversation delta, so the
 * card and its analytics row are tagged with the right backend from the start.
 */
export async function spawnManagedAgent(opts: ManagedSpawnOptions): Promise<string> {
  const { provider } = opts;
  // Supervisors with no explicit cwd open in their dedicated home (~/.workspacer)
  // rather than inheriting some repo; everything else uses the given cwd.
  let cwd = opts.cwd || process.env.HOME || os.homedir();
  if (opts.supervisor && !opts.cwd) cwd = ensureSupervisorHome();

  const bin = resolveAgentBinary(provider);
  const wantsFacade = opts.supervisor || opts.mcpFacade;
  const managedId = opts.resumeSessionId || randomUUID();
  claudeSessionStore.setSpawnMeta(managedId, {
    label: opts.label,
    parentSessionId: opts.parentSessionId,
    isSupervisor: opts.supervisor,
    provider,
  });
  const sessionId = await claudemonSessionClient.spawnManaged({
    provider,
    cwd,
    model: opts.model,
    bin,
    yolo: opts.skipPermissions,
    sessionId: managedId,
    ...(wantsFacade && {
      mcp: MCP_FACADE_URL,
      instructions: managedFacadeInstructions(!!opts.supervisor),
    }),
  });
  // The adapter emits no conversation delta until the agent first produces
  // output, and managed backends fire no Claude hooks — so register the session
  // now, or its GUI pane would sit on the empty "connecting" state (showing
  // "no session") until the first message. The conversation/statusline streams
  // enrich this entry as the agent runs.
  claudeSessionStore.ensureManagedSession(sessionId, cwd);
  return sessionId;
}
