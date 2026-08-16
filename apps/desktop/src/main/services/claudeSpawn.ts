/**
 * Shared Claude (Tier-1, PTY) spawn dispatch.
 *
 * Claude Code runs as a real `claude` process in a PTY (unlike the managed
 * adapters in managedSpawn.ts). Two entry points start one — the desktop
 * `claude:spawn` IPC handler and the `agents.spawn` hub-bus capability (web /
 * remote client + MCP facade) — and this helper is the single body they both
 * call so they can't drift.
 *
 * The bug it fixes: the hub copy was a hand-maintained duplicate whose comment
 * claimed it "Mirrors the IPC path exactly", but it had silently fallen behind —
 * it never built the per-spawn Library MCP config, so a Claude agent started
 * from the web/remote/MCP path lost its selected `mcpItemIds` (no --mcp-config,
 * no pre-allowed tools). Centralising here makes the two paths identical: MCP
 * servers apply whenever `mcpItemIds` is present, on either transport.
 *
 * Callers own their own policy *before* calling: the hub path sanitises the
 * permission bypass (a remote caller may not silently auto-approve) and passes
 * the already-safe `skipPermissions` / `permissionMode` in.
 */
import * as os from 'os';
import { normalizeSpawnCwd } from '../lib/spawnCwd';
import * as fs from 'fs';
import { randomUUID } from 'crypto';
import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { claudeProfiles, scrubBypassProfile } from './claudeProfiles';
import { resolveClaudeDefaultEffort } from './claudeEffortDefault';
import { buildClaudeArgv } from './claudeResolver';
import { claudemonOverlayPath, claudeSettingsOverlayEnabled } from './claudemonDaemon';
import { facadeSpawnArgs, buildSessionMcpConfig } from './mcpConfig';
import { libraryService } from './libraryService';
import { configService } from './configService';
import { installSupervisorSkill, ensureSupervisorHome } from './supervisorSkill';
import { mintSessionFacadeToken } from './remoteTokens';
import type { RemoteTokenScope } from '../shared/ipcTypes';

export interface ClaudeSpawnOptions {
  cwd?: string;
  /** Claude profile (CLAUDE_CONFIG_DIR + extraArgs). */
  profileId?: string;
  model?: string;
  /** Reasoning-effort level (`--effort <level>`). Re-passed on respawn. */
  effort?: string;
  /**
   * Explicit Claude permission mode. When omitted, `skipPermissions` maps to
   * 'bypassPermissions' and everything else to 'default' — same resolution the
   * old inline IPC path used.
   */
  permissionMode?: string;
  /** Strip any permission bypass the chosen PROFILE carries. Set by callers on
   *  an untrusted boundary (the hub/remote spawn capability): clamping the
   *  request's own fields is not enough when a profile can smuggle the same
   *  flag in through extraArgs. */
  scrubProfileBypass?: boolean;
  /** YOLO / `--dangerously-skip-permissions`. */
  skipPermissions?: boolean;
  /** Re-use this id (resume an existing session). */
  resumeSessionId?: string;
  /** Wire the workspacer MCP facade + run the /supervise loop. */
  supervisor?: boolean;
  /** Wire the facade tools without the supervisor loop (legacy operator tier —
   *  prefer `toolScope`). */
  mcpFacade?: boolean;
  /**
   * Grant the workspacer facade tools at a TIER: 'view' (observe-only — right
   * for summarizer workers), 'triage' (view + approve/reply/interrupt), or
   * 'operator' (everything). Mints a per-session scoped token the facade
   * enforces, so the agent sees (and pays context for) only its tier's tools.
   * Implies the facade; `supervisor`/`mcpFacade` without it mean 'operator'.
   */
  toolScope?: RemoteTokenScope;
  /** Plugin ids whose contributed facade tools this session may use (opt-in;
   *  recorded on the session token — see authtoken.Record.Plugins). */
  pluginTools?: string[];
  label?: string;
  parentSessionId?: string;
  cols?: number;
  rows?: number;
  /**
   * Library item ids (kind 'mcp') selected for this spawn. Resolved to a
   * session-scoped `--mcp-config` with `--strict-mcp-config` + pre-allowed
   * tools. Ignored for facade/supervisor sessions (they take the facade config).
   */
  mcpItemIds?: string[];
}

/**
 * Spawn a Claude Code PTY session and return its session id. Pins the id so
 * claude names its transcript `<id>.jsonl` (our id == claude's id == filename),
 * records spawn metadata before the first hook event, and applies per-spawn
 * Library MCP servers when `mcpItemIds` is present.
 */
export async function spawnClaudeAgent(opts: ClaudeSpawnOptions): Promise<string> {
  const rawProfile = opts.profileId ? claudeProfiles.getProfile(opts.profileId) : undefined;
  const profile = opts.scrubProfileBypass ? scrubBypassProfile(rawProfile) : rawProfile;
  const env: Record<string, string> = {};
  if (profile?.configDir) {
    env.CLAUDE_CONFIG_DIR = profile.configDir.replace(/^~/, os.homedir());
  }
  // Pin the session id so claude names its transcript `<id>.jsonl` and our
  // id == claude's id == the filename. Resuming keeps the existing id.
  const sessionId = opts.resumeSessionId || randomUUID();
  // An explicit mode wins; the legacy boolean maps to bypass. Recorded on the
  // snapshot so the composer pill shows truth.
  const permissionMode =
    opts.permissionMode ?? (opts.skipPermissions ? 'bypassPermissions' : 'default');
  // Whether this process will carry `--dangerously-skip-permissions` — the same
  // three inputs buildClaudeArgv resolves it from below. Recorded because Claude
  // gates *switching to* bypassPermissions on the flag, so it's what tells the
  // composer whether "Full access" is a live switch or a restart.
  const bypassAvailable =
    !!opts.skipPermissions ||
    permissionMode === 'bypassPermissions' ||
    (profile?.extraArgs ?? []).includes('--dangerously-skip-permissions');
  // Record name/parent before the session registers so adopted cards are
  // enriched from the very first hook event.
  claudeSessionStore.setSpawnMeta(sessionId, {
    label: opts.label,
    parentSessionId: opts.parentSessionId,
    isSupervisor: opts.supervisor,
    provider: 'claude',
    settings: {
      model: opts.model,
      effort: opts.effort,
      permissionMode,
      bypassAvailable,
      // What an absent `--effort` resolves to, so the pill can name the level
      // instead of the word "Default". The CLI reports it nowhere.
      ...(!opts.effort?.trim() && {
        defaultEffort: resolveClaudeDefaultEffort(opts.cwd, profile?.configDir),
      }),
    },
  });

  // Per-spawn MCP servers selected from the Library (kind 'mcp'). Resolve the
  // chosen item ids to their configs, write a session-scoped --mcp-config, and
  // pre-allow their tools. `--strict-mcp-config` so the session sees exactly
  // these servers, not the user's global ones. Sessions with the workspacer MCP
  // facade (full supervisors, or plain facade workers a supervisor spawns) take
  // the facade config instead of the user's library MCP servers.
  const wantsFacade = opts.supervisor || opts.mcpFacade || !!opts.toolScope;
  // A supervisor is operator by definition; a plain facade session takes its
  // requested tier, defaulting to operator (the legacy mcpFacade meaning).
  const facadeScope: RemoteTokenScope = opts.supervisor
    ? 'operator'
    : (opts.toolScope ?? 'operator');
  let userMcp: { path: string; toolNames: string[] } | null = null;
  if (!wantsFacade && opts.mcpItemIds && opts.mcpItemIds.length) {
    const wanted = new Set(opts.mcpItemIds);
    // listWithSecrets, not list(): the config written below is what the CLI
    // actually authenticates with, and list() masks MCP env/headers. The real
    // values never leave main — the renderer sent only `mcpItemIds`.
    const servers = libraryService
      .listWithSecrets(opts.cwd)
      .filter((it) => it.kind === 'mcp' && it.mcp && wanted.has(it.id))
      .map((it) => ({ id: it.id, mcp: it.mcp! }));
    userMcp = buildSessionMcpConfig(sessionId, servers);
  }

  // Supervisors: install the /supervise skill and default to the configured
  // supervisor model when none was passed explicitly.
  const supCfg = configService.getConfig().supervisor;
  let model = opts.model;
  if (opts.supervisor) {
    installSupervisorSkill();
    if (!model) model = supCfg?.model || undefined;
  }

  const argv = buildClaudeArgv({
    extraArgs: profile?.extraArgs,
    resumeSessionId: opts.resumeSessionId,
    model,
    effort: opts.effort,
    settingsFile: claudeSettingsOverlayEnabled() ? claudemonOverlayPath() : undefined,
    skipPermissions: opts.skipPermissions,
    permissionMode: permissionMode as 'default' | 'acceptEdits' | 'plan' | 'bypassPermissions',
    sessionId,
    // Facade sessions get the MCP config + pre-allowed tools + a role prompt.
    // A supervisor also learns its session id and is kicked into /supervise;
    // a plain facade worker just gets the tools. The per-session token pins
    // the tier server-side — the facade refuses calls outside it even if the
    // agent guesses tool names.
    ...(wantsFacade &&
      facadeSpawnArgs({
        sessionId,
        supervisor: opts.supervisor,
        scope: facadeScope,
        token: mintSessionFacadeToken(sessionId, facadeScope, opts.pluginTools).token,
        summarizerModel: supCfg?.summarizerModel,
        pollSeconds: supCfg?.pollSeconds,
      })),
    // User-selected MCP servers (non-facade sessions).
    ...(userMcp && {
      mcpConfig: userMcp.path,
      strictMcpConfig: true,
      allowedTools: userMcp.toolNames,
    }),
  });
  // Fleet supervisors with no explicit cwd open in their dedicated home
  // (~/.workspacer); everything else uses the given cwd (when it exists) or
  // falls back to home. The existence guard tolerates a stale/bad path from a
  // remote caller instead of failing the spawn.
  let cwd = normalizeSpawnCwd(opts.cwd);
  if (opts.supervisor && !opts.cwd) cwd = ensureSupervisorHome();
  return claudemonSessionClient.spawn({
    argv,
    cwd,
    cols: opts.cols,
    rows: opts.rows,
    env,
    sessionId,
  });
}
