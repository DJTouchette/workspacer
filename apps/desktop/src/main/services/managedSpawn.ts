/**
 * Shared managed-provider (Tier-2) spawn dispatch.
 *
 * OpenCode / Codex / Pi are driven by claudemon's adapters (their own machine
 * interface — `opencode serve`, `codex app-server`, …) rather than a PTY, so
 * spawning one is `POST /sessions/spawn-managed`, not the Claude `argv` spawn.
 * Claude itself also has a managed form: the 'stream' transport runs headless
 * `claude --print --input-format stream-json --output-format stream-json`
 * through claudemon's claude_stream adapter (no PTY, GUI-only pane). The
 * classic PTY transport still goes through claudeSpawn.ts.
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
import { claudeProfiles } from './claudeProfiles';
import { libraryService } from './libraryService';
import { resolveAgentBinary, isAgentBinaryInstalled, type AgentProvider } from './agentProviders';
import { configService } from './configService';
import { MCP_FACADE_URL, managedFacadeInstructions, buildSessionMcpConfig } from './mcpConfig';
import { ensureSupervisorHome } from './supervisorSkill';
import { notifySystem } from './systemNotice';

/** Install hints surfaced when a provider CLI isn't on PATH. */
const INSTALL_HINT: Record<AgentProvider, string> = {
  claude: 'Install Claude Code and make sure `claude` is on your PATH.',
  codex: 'Install the Codex CLI and make sure `codex` is on your PATH.',
  opencode: 'Install OpenCode and make sure `opencode` is on your PATH.',
  pi: 'Install Pi and make sure `pi` is on your PATH.',
};

/** User-configured binary path for a provider ('' if not set). */
function configuredBin(provider: AgentProvider): string {
  return configService.getConfig().agents?.binaries?.[provider] ?? '';
}

/** Pre-flight: fail fast with a clear banner if the provider CLI is missing,
 *  rather than spawning a process that dies with an opaque ENOENT. */
function assertProviderInstalled(provider: AgentProvider): void {
  if (isAgentBinaryInstalled(provider, configuredBin(provider))) return;
  const title = `${provider} CLI not found`;
  notifySystem({
    level: 'error',
    key: `missing-${provider}`,
    title,
    detail: INSTALL_HINT[provider],
  });
  throw new Error(`${title}. ${INSTALL_HINT[provider]}`);
}

export interface ManagedSpawnOptions {
  /** The managed backend to launch. 'claude' is only valid together with
   *  `transport: 'stream'` (the headless stream-json adapter) — PTY Claude
   *  spawns are dispatched to spawnClaudeAgent by the caller instead. */
  provider: AgentProvider;
  /** Claude only: must be 'stream' when provider === 'claude'. */
  transport?: 'stream';
  cwd?: string;
  model?: string;
  /** Reasoning-effort level (codex `model_reasoning_effort`); others ignore it. */
  effort?: string;
  /** YOLO / auto-approve every command and file change. */
  skipPermissions?: boolean;
  /** Claude (stream) only: explicit permission mode
   *  (default/acceptEdits/plan/bypassPermissions). Managed providers use the
   *  ask/yolo pair via `skipPermissions`. */
  permissionMode?: string;
  /** Claude (stream) only: Claude profile (CLAUDE_CONFIG_DIR + extraArgs) —
   *  same semantics as the PTY path (claudeSpawn.ts). */
  profileId?: string;
  /** Claude (stream) only: Library item ids (kind 'mcp') selected for this
   *  spawn, resolved to a session-scoped `--mcp-config` with
   *  `--strict-mcp-config` + pre-allowed tools — same as the PTY path. */
  mcpItemIds?: string[];
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
  // Managed Claude exists only as the stream-json transport; a PTY Claude spawn
  // must never land here (the callers dispatch it to spawnClaudeAgent).
  if (provider === 'claude' && opts.transport !== 'stream') {
    throw new Error("spawnManagedAgent: provider 'claude' requires transport 'stream'");
  }
  assertProviderInstalled(provider);
  // Codex is a hybrid (GUI + Term) on every platform, but the wiring differs:
  //  - macOS/Linux: the app-server JSON-RPC adapter (the generic managed path
  //    below) drives the structured GUI *and* spawns the native TUI in a PTY,
  //    resumed onto the same live app-server thread over `--remote ws://…` — so
  //    claudemon owns both surfaces of one session (see providers/codex.rs).
  //  - Windows: the older rollout-tail hybrid — its own TUI runs in a PTY and
  //    claudemon tails the rollout transcript for the GUI. Kept until the ws
  //    app-server path is verified on Windows, at which point this branch (and
  //    the rollout tailer) can go and all platforms share the managed path.
  if (provider === 'codex' && process.platform === 'win32') return spawnCodexHybrid(opts);
  // Supervisors with no explicit cwd open in their dedicated home (~/.workspacer)
  // rather than inheriting some repo; everything else uses the given cwd.
  let cwd = opts.cwd || process.env.HOME || os.homedir();
  if (opts.supervisor && !opts.cwd) cwd = ensureSupervisorHome();

  const isClaudeStream = provider === 'claude';
  const bin = resolveAgentBinary(provider, configuredBin(provider));
  const wantsFacade = opts.supervisor || opts.mcpFacade;
  const managedId = opts.resumeSessionId || randomUUID();
  // Permission-mode vocabulary differs by family: Claude keeps its full mode
  // set (an explicit mode wins; the legacy boolean maps to bypass — same
  // resolution as the PTY path), managed providers are just ask/yolo.
  const permissionMode = isClaudeStream
    ? (opts.permissionMode ?? (opts.skipPermissions ? 'bypassPermissions' : 'default'))
    : opts.skipPermissions
      ? 'yolo'
      : 'ask';
  const yolo = isClaudeStream
    ? opts.skipPermissions || permissionMode === 'bypassPermissions'
    : opts.skipPermissions;
  // Claude (stream) parity with the PTY path (claudeSpawn.ts): a profile maps
  // to CLAUDE_CONFIG_DIR + its extra argv, and Library MCP selections become a
  // session-scoped --mcp-config with --strict-mcp-config + pre-allowed tools.
  // Both ride the spawn-managed payload's claude-only env/extra_args fields
  // instead of being silently dropped. Facade sessions take the facade MCP
  // config instead of the user's library servers, as on the PTY path.
  const profile =
    isClaudeStream && opts.profileId ? claudeProfiles.getProfile(opts.profileId) : undefined;
  const env: Record<string, string> = {};
  if (profile?.configDir) {
    env.CLAUDE_CONFIG_DIR = profile.configDir.replace(/^~/, os.homedir());
  }
  const extraArgs: string[] = [...(profile?.extraArgs ?? [])];
  if (isClaudeStream && !wantsFacade && opts.mcpItemIds && opts.mcpItemIds.length) {
    const wanted = new Set(opts.mcpItemIds);
    const servers = libraryService
      .list(opts.cwd)
      .filter((it) => it.kind === 'mcp' && it.mcp && wanted.has(it.id))
      .map((it) => ({ id: it.id, mcp: it.mcp! }));
    const userMcp = buildSessionMcpConfig(managedId, servers);
    if (userMcp) {
      extraArgs.push('--mcp-config', userMcp.path, '--strict-mcp-config');
      if (userMcp.toolNames.length) {
        extraArgs.push('--allowedTools', userMcp.toolNames.join(','));
      }
    }
  }
  claudeSessionStore.setSpawnMeta(managedId, {
    label: opts.label,
    parentSessionId: opts.parentSessionId,
    isSupervisor: opts.supervisor,
    provider,
    ...(isClaudeStream && { transport: 'stream' as const }),
    settings: {
      model: opts.model,
      effort: opts.effort,
      permissionMode,
    },
  });
  const sessionId = await claudemonSessionClient.spawnManaged({
    provider,
    cwd,
    model: opts.model,
    effort: opts.effort,
    bin,
    yolo,
    sessionId: managedId,
    // Claude stream adapter extras: the full permission mode and (on a
    // respawn) the prior conversation to `--resume`.
    ...(isClaudeStream && {
      permissionMode,
      ...(opts.resumeSessionId && { resumeSessionId: opts.resumeSessionId }),
      ...(extraArgs.length && { extraArgs }),
      ...(Object.keys(env).length && { env }),
    }),
    ...(wantsFacade && {
      mcp: MCP_FACADE_URL,
      instructions: managedFacadeInstructions(!!opts.supervisor),
    }),
  });
  // The adapter emits no conversation delta until the agent first produces
  // output, and managed backends fire no Claude hooks — so register the session
  // now, or its GUI pane would sit on the empty "connecting" state (showing
  // "no session") until the first message. The conversation/statusline streams
  // enrich this entry as the agent runs. (Stream-transport Claude *does* fire
  // hooks, but only after the first turn starts — same gap, same fix.)
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
  const bin = resolveAgentBinary('codex', configuredBin('codex'));
  const sessionId = opts.resumeSessionId || randomUUID();
  claudeSessionStore.setSpawnMeta(sessionId, {
    label: opts.label,
    parentSessionId: opts.parentSessionId,
    isSupervisor: opts.supervisor,
    provider: 'codex',
    settings: {
      model: opts.model,
      effort: opts.effort,
      permissionMode: opts.skipPermissions ? 'yolo' : 'ask',
    },
  });
  // Show the card immediately; the rollout tailer + conversation stream enrich it.
  claudeSessionStore.ensureManagedSession(sessionId, cwd);
  // Codex takes model/effort overrides as config flags (`-c model="<id>"`,
  // `-c model_reasoning_effort=<level>`); YOLO maps to bypassing its
  // approval/sandbox prompts so the TUI doesn't block on them.
  const model = opts.model?.trim();
  const effort = opts.effort?.trim();
  const argv = [
    bin,
    ...(model ? ['-c', `model=${JSON.stringify(model)}`] : []),
    ...(effort ? ['-c', `model_reasoning_effort=${JSON.stringify(effort)}`] : []),
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
