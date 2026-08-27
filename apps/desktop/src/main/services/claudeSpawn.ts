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
import { assertSpawnCwd, normalizeSpawnCwd } from '../lib/spawnCwd';
import * as fs from 'fs';
import { randomUUID } from 'crypto';
import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { claudeProfiles, scrubBypassProfile, scrubRemoteGrantedProfile } from './claudeProfiles';
import { syncAccountTrust } from './claudeAccountSetup';
import { resolveClaudeDefaultEffort } from './claudeEffortDefault';
import { buildClaudeArgv } from './claudeResolver';
import { claudemonOverlayPath, claudeSettingsOverlayEnabled } from './claudemonDaemon';
import { facadeSpawnArgs, buildSessionMcpConfig } from './mcpConfig';
import { libraryService } from './libraryService';
import { configService } from './configService';
import { resolveSpawnModel } from '../lib/spawnModel';
import { installSupervisorSkill, ensureSupervisorHome } from './supervisorSkill';
import { installManagerSkills } from './managerSkills';
import { mintSessionFacadeToken } from './remoteTokens';
import { managerFullAccessFromConfig } from './fullAccessGrants';
import { buildResultContract, checkResultSchema } from '../shared/structuredResult';
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
  /** The hub verified this spawn's caller holds a token whose profilesAllowed
   *  grant names this profileId (fleet-manager dispatch). Softens the scrub to
   *  scrubRemoteGrantedProfile — configDir kept, bypass args and mcpItemIds
   *  still stripped. Only hubCapabilities sets it, from the hub-STAMPED param
   *  (sanitizeSpawnParams deletes any caller-supplied copy). Meaningless
   *  without scrubProfileBypass. */
  profileGranted?: boolean;
  /** YOLO / `--dangerously-skip-permissions`. */
  skipPermissions?: boolean;
  /** Re-use this id (resume an existing session). */
  resumeSessionId?: string;
  /** Wire the workspacer MCP facade + run the /supervise loop. */
  supervisor?: boolean;
  /** Fleet Manager: nudge-eligible parent (isSupervisor spawn meta) without
   *  the /supervise loop — see managedSpawn's twin field. */
  manager?: boolean;
  /** Manager full-access HINT from the caller. The token's actual yolo grant
   *  is config-resolved at mint (services/fullAccessGrants — same doctrine as
   *  supervisor.fullAccess), so this flag no longer decides anything here; it
   *  is kept on the wire for record fidelity (the renderer persists it on the
   *  agent card and re-passes it on respawn). */
  fleetFullAccess?: boolean;
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
  /**
   * Structured-result contract: a JSON Schema the dispatcher wants the worker's
   * final report to carry as a fenced `wks-result` block. Compiled into
   * `--append-system-prompt` here and validated at the worker-finished wake
   * (shared/structuredResult, supervisorNudge). Purely additive — the worker
   * still writes its prose summary, and a botched block degrades to a reported
   * `resultError` beside it.
   */
  resultSchema?: Record<string, unknown>;
  /**
   * The agent's FIRST PROMPT — the dispatch itself — carried by the spawn
   * instead of by a separate `claude.message` / `agents.sendMessage` once the
   * id comes back.
   *
   * Two-call dispatch has a real window: the daemon registers a session id and
   * answers 200 BEFORE the child is up, so the caller is handed an addressable
   * id for a session that cannot yet take input (managed rows refuse with 404;
   * a PTY has to wait for its first `Input` transition either way). Riding the
   * spawn payload removes the window — claudemon queues the prompt inside the
   * spawn handler and delivers it through the same settle-and-verify ladder a
   * chat send uses.
   *
   * NOT the same channel as the result contract: that is compiled into
   * `--append-system-prompt` here (a system prompt, always present), while this
   * is a user turn (it starts the work). Both reach the worker, in that order.
   */
  firstMessage?: string;
}

/**
 * Spawn a Claude Code PTY session and return its session id. Pins the id so
 * claude names its transcript `<id>.jsonl` (our id == claude's id == filename),
 * records spawn metadata before the first hook event, and applies per-spawn
 * Library MCP servers when `mcpItemIds` is present.
 */
export async function spawnClaudeAgent(opts: ClaudeSpawnOptions): Promise<string> {
  const rawProfile = opts.profileId ? claudeProfiles.getProfile(opts.profileId) : undefined;
  const profile = opts.scrubProfileBypass
    ? opts.profileGranted
      ? scrubRemoteGrantedProfile(rawProfile)
      : scrubBypassProfile(rawProfile)
    : rawProfile;
  const env: Record<string, string> = {};
  if (profile?.configDir) {
    env.CLAUDE_CONFIG_DIR = profile.configDir.replace(/^~/, os.homedir());
  }
  // Pin the session id so claude names its transcript `<id>.jsonl` and our
  // id == claude's id == the filename. Resuming keeps the existing id.
  const sessionId = opts.resumeSessionId || randomUUID();
  // A malformed/oversized result contract is refused OUT LOUD rather than
  // dropped: the caller asked for a machine-readable report, and a spawn that
  // silently forgets the contract would hand it prose it did not expect.
  const resultSchema = opts.resultSchema;
  if (resultSchema !== undefined) {
    const bad = checkResultSchema(resultSchema);
    if (bad) throw new Error(`spawn: ${bad}`);
  }
  // Supervisor full-access mode (config supervisor.fullAccess, the supervisor
  // twin of agents.fleetFullAccess): the supervisor itself runs with
  // permissions bypassed, and its facade token below is minted with the yolo
  // grant so the workers it spawns may run bypassed too. Read from config —
  // not a caller flag — so every entry point (IPC, hub bus, jobs) resolves the
  // local user's setting identically.
  const supCfg = configService.getConfig().supervisor;
  const supervisorFullAccess = !!opts.supervisor && supCfg?.fullAccess === true;
  const skipPermissions = !!opts.skipPermissions || supervisorFullAccess;
  // An explicit mode wins; the legacy boolean maps to bypass (and supervisor
  // full access forces it). Recorded on the snapshot so the composer pill
  // shows truth.
  const permissionMode = supervisorFullAccess
    ? 'bypassPermissions'
    : (opts.permissionMode ?? (skipPermissions ? 'bypassPermissions' : 'default'));
  // Whether this process will carry `--dangerously-skip-permissions` — the same
  // three inputs buildClaudeArgv resolves it from below. Recorded because Claude
  // gates *switching to* bypassPermissions on the flag, so it's what tells the
  // composer whether "Full access" is a live switch or a restart.
  const bypassAvailable =
    skipPermissions ||
    permissionMode === 'bypassPermissions' ||
    (profile?.extraArgs ?? []).includes('--dangerously-skip-permissions');
  // Record name/parent before the session registers so adopted cards are
  // enriched from the very first hook event.
  claudeSessionStore.setSpawnMeta(sessionId, {
    label: opts.label,
    parentSessionId: opts.parentSessionId,
    isSupervisor: opts.supervisor || opts.manager,
    provider: 'claude',
    ...(resultSchema && { resultSchema }),
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
  let model = opts.model;
  if (opts.supervisor) {
    installSupervisorSkill();
    if (!model) model = supCfg?.model || undefined;
  }
  // Then the general default, so an omitted model is RESOLVED rather than left
  // to Claude Code's own internal choice. It goes on the argv AND (below) into
  // the spawn payload, which is what lets the daemon know this session's window
  // from token zero instead of guessing 200k off a marker-stripped transcript
  // id. See lib/spawnModel.
  model = resolveSpawnModel('claude', model);
  // The Fleet Manager's invocable skills (/bearings, /stow) — parity with the
  // stream path (managedSpawn), where the manager normally runs.
  if (opts.manager) {
    installManagerSkills();
  }

  // The facade fragment is built BEFORE the argv so the structured-result
  // contract can be appended to its --append-system-prompt instead of racing it
  // for the single flag: buildClaudeArgv takes one appendSystemPrompt, and a
  // second spread would silently drop whichever key lost. A non-facade worker
  // (the common ship-task dispatch) gets the contract as its only appended
  // prompt.
  const facadeArgs =
    wantsFacade &&
    facadeSpawnArgs({
      sessionId,
      supervisor: opts.supervisor,
      scope: facadeScope,
      // A host-blessed Fleet Manager's token carries a dispatch grant for
      // every local profile — the hub verifies it and stamps profileGranted
      // on the worker spawn. Only `manager` gets this; a plain supervisor
      // or facade worker has no business spawning as other accounts.
      // The yolo grant is CONFIG-RESOLVED for both roles (fleet full access /
      // per-project yolo for the manager, supervisor.fullAccess for a
      // supervisor — services/fullAccessGrants is the single formula), never
      // a caller flag: a respawn re-passing a value frozen at the original
      // spawn must not resurrect a grant the user has since revoked, nor
      // withhold one they granted. The hub then stamps yoloGranted on the
      // holder's worker spawns so their skipPermissions request is honored
      // instead of clamped. The role tag is what lets a later config flip
      // find this token and update the grant LIVE (fullAccessGrants sync).
      token: mintSessionFacadeToken(
        sessionId,
        facadeScope,
        opts.pluginTools,
        opts.manager ? claudeProfiles.getProfiles().map((p) => p.id) : undefined,
        opts.manager ? managerFullAccessFromConfig() : supervisorFullAccess || undefined,
        opts.manager ? 'manager' : opts.supervisor ? 'supervisor' : undefined,
      ).token,
      summarizerModel: supCfg?.summarizerModel,
      pollSeconds: supCfg?.pollSeconds,
      fullAccess: supervisorFullAccess,
    });
  const contract = resultSchema ? buildResultContract(resultSchema) : '';
  const appendSystemPrompt = [facadeArgs ? facadeArgs.appendSystemPrompt : '', contract]
    .filter(Boolean)
    .join('\n\n');

  const argv = buildClaudeArgv({
    extraArgs: profile?.extraArgs,
    resumeSessionId: opts.resumeSessionId,
    model,
    effort: opts.effort,
    settingsFile: claudeSettingsOverlayEnabled() ? claudemonOverlayPath() : undefined,
    skipPermissions,
    permissionMode: permissionMode as 'default' | 'acceptEdits' | 'plan' | 'bypassPermissions',
    sessionId,
    // Facade sessions get the MCP config + pre-allowed tools + a role prompt.
    // A supervisor also learns its session id and is kicked into /supervise;
    // a plain facade worker just gets the tools. The per-session token pins
    // the tier server-side — the facade refuses calls outside it even if the
    // agent guesses tool names. Built above so the structured-result contract
    // can share the one --append-system-prompt.
    ...(facadeArgs && { mcpConfig: facadeArgs.mcpConfig, allowedTools: facadeArgs.allowedTools }),
    ...(appendSystemPrompt && { appendSystemPrompt }),
    // User-selected MCP servers (non-facade sessions).
    ...(userMcp && {
      mcpConfig: userMcp.path,
      strictMcpConfig: true,
      allowedTools: userMcp.toolNames,
    }),
  });
  // Fleet supervisors with no explicit cwd open in their dedicated home
  // (~/.workspacer); everything else uses the given cwd exactly as written —
  // normalizeSpawnCwd trims and nothing more, deliberately (BINDING DECISION 1:
  // no layer on a caller's path expands '~'). Which is why the pre-flight below
  // has to exist: a path that cannot be a working directory must fail HERE,
  // where the user is told, rather than as a session claudemon registers and
  // then stops the instant the child fails to launch.
  let cwd = normalizeSpawnCwd(opts.cwd);
  if (opts.supervisor && !opts.cwd) cwd = ensureSupervisorHome();
  assertSpawnCwd(cwd);
  // A profile spawn inherits the primary login's trust for this folder, or a
  // PTY parks on the invisible trust dialog (mode "unknown", dead pane).
  if (env.CLAUDE_CONFIG_DIR) syncAccountTrust(env.CLAUDE_CONFIG_DIR, cwd);
  return claudemonSessionClient.spawn({
    argv,
    cwd,
    cols: opts.cols,
    rows: opts.rows,
    env,
    sessionId,
    // Explicitly, not only via `--model` on the argv: a resume re-uses the
    // prior life's model without re-stating it, and the daemon's argv sniffing
    // would find nothing to record for exactly the sessions that have the most
    // history to mis-measure.
    model,
    firstMessage: opts.firstMessage,
  });
}
