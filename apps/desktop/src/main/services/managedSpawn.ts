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
import { resolveAgentBinary, isAgentBinaryInstalled, type AgentProvider } from './agentProviders';
import { MCP_FACADE_URL, managedFacadeInstructions } from './mcpConfig';
import { ensureSupervisorHome } from './supervisorSkill';
import { notifySystem } from './systemNotice';

/** Install hints surfaced when a provider CLI isn't on PATH. */
const INSTALL_HINT: Record<Exclude<AgentProvider, 'claude'>, string> = {
  codex: 'Install the Codex CLI and make sure `codex` is on your PATH.',
  opencode: 'Install OpenCode and make sure `opencode` is on your PATH.',
  pi: 'Install Pi and make sure `pi` is on your PATH.',
};

/** Pre-flight: fail fast with a clear banner if the provider CLI is missing,
 *  rather than spawning a process that dies with an opaque ENOENT. */
function assertProviderInstalled(provider: Exclude<AgentProvider, 'claude'>): void {
  if (isAgentBinaryInstalled(provider)) return;
  const title = `${provider} CLI not found`;
  notifySystem({ level: 'error', key: `missing-${provider}`, title, detail: INSTALL_HINT[provider] });
  throw new Error(`${title}. ${INSTALL_HINT[provider]}`);
}

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
  /** PTY dimensions for hybrid (PTY-backed) providers like Codex. */
  cols?: number;
  rows?: number;
}

/**
 * Spawn a managed (adapter-driven) agent session and return its session id.
 * Mirrors the Claude path's pre-registration: a pinned id + spawn metadata
 * (provider/label/parent) recorded before the first conversation delta, so the
 * card and its analytics row are tagged with the right backend from the start.
 */
export async function spawnManagedAgent(opts: ManagedSpawnOptions): Promise<string> {
  const { provider } = opts;
  assertProviderInstalled(provider);
  // Codex backend differs by platform:
  //  - Windows: HYBRID — its own TUI runs in a PTY (the Term view) and claudemon
  //    tails the rollout transcript for the GUI. The Codex app-server *daemon*
  //    that would let a TUI and an RPC client share one live thread is Unix-only,
  //    so the rollout is the only live structured channel out of the TUI here.
  //  - macOS/Linux: the app-server JSON-RPC adapter (the generic managed path
  //    below) drives a structured GUI directly.
  if (provider === 'codex' && process.platform === 'win32') return spawnCodexHybrid(opts);
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

/**
 * Spawn a hybrid Codex agent: the `codex` TUI runs in a PTY (the Term view,
 * via the normal `/sessions/spawn`), and claudemon additionally tails Codex's
 * rollout transcript (`rolloutProvider: 'codex'`) to populate the GUI
 * conversation view from the same live session. Returns the canonical session
 * id (pinned so the daemon's id == ours).
 */
async function spawnCodexHybrid(opts: ManagedSpawnOptions): Promise<string> {
  let cwd = opts.cwd || process.env.HOME || os.homedir();
  if (opts.supervisor && !opts.cwd) cwd = ensureSupervisorHome();
  const bin = resolveAgentBinary('codex');
  const sessionId = opts.resumeSessionId || randomUUID();
  claudeSessionStore.setSpawnMeta(sessionId, {
    label: opts.label,
    parentSessionId: opts.parentSessionId,
    isSupervisor: opts.supervisor,
    provider: 'codex',
  });
  // Show the card immediately; the rollout tailer + conversation stream enrich it.
  claudeSessionStore.ensureManagedSession(sessionId, cwd);
  // Codex takes a model override as a config flag (`-c model="<id>"`); YOLO maps
  // to bypassing its approval/sandbox prompts so the TUI doesn't block on them.
  const model = opts.model?.trim();
  const argv = [
    bin,
    ...(model ? ['-c', `model=${JSON.stringify(model)}`] : []),
    ...(opts.skipPermissions ? ['--dangerously-bypass-approvals-and-sandbox'] : []),
  ];
  await claudemonSessionClient.spawn({
    argv,
    cwd,
    cols: opts.cols ?? 120,
    rows: opts.rows ?? 32,
    sessionId,
    rolloutProvider: 'codex',
  });
  return sessionId;
}
