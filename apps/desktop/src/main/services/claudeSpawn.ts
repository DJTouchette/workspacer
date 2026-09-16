import { managerReplacementState } from './managerReplacementState';
import { prepareLaunchIntegration } from './launchIntegrations';
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
import { claudeSessionStore, type SessionRouting } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { claudeProfiles } from './claudeProfiles';
import { syncAccountTrust } from './claudeAccountSetup';
import { resolveClaudeDefaultEffort } from './claudeEffortDefault';
import { buildClaudeArgv, modelFromExtraArgs } from './claudeResolver';
import { claudemonOverlayPath, claudeSettingsOverlayEnabled } from './claudemonDaemon';
import { facadeSpawnArgs, type SessionMcpServer } from './mcpConfig';
import { libraryService } from './libraryService';
import { configService } from './configService';
import { resolveSpawnModelSelection } from '../lib/spawnModel';
import {
  resolveManagerContextForSpawn,
  resolveManagerModel,
  resolveManagerEffort,
} from '../lib/roleModels';
import { installManagerSkills } from './managerSkills';
import { installResponseCardSkill } from './responseCardSkill';
import { installAgentCollaborationSkills } from './agentCollaborationSkills';
import { mintSessionFacadeToken, revokeSessionFacadeTokens } from './remoteTokens';
import { ensureMcpFacadeReady } from './mcpFacadeDaemon';
import { buildResultContract, checkResultSchema } from '../shared/structuredResult';
import { buildWorkerEscalationContract, isFleetDispatchedWorker } from '../shared/workerEscalation';
import { profileAppliesTo } from '../shared/agentProfiles';
import type { RemoteTokenScope } from '../shared/ipcTypes';
import { claudeArgvModel } from '../shared/modelContextWindows';

export interface ClaudeSpawnOptions {
  cwd?: string;
  /** Claude profile (CLAUDE_CONFIG_DIR + extraArgs). */
  profileId?: string;
  launchIntegrationId?: string | null;
  model?: string;
  /** Additive canonical pair; `model` stays the executable legacy companion. */
  modelIdentity?: string;
  contextWindow?: number | null;
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
  /** Legacy compatibility field; profile grants are not enforced. */
  profileGranted?: boolean;
  /** YOLO / `--dangerously-skip-permissions`. */
  skipPermissions?: boolean;
  /** Re-use this id (resume an existing session). */
  resumeSessionId?: string;
  /** Fleet Manager: nudge-eligible parent (isWakeTarget spawn meta) — see
   *  managedSpawn's twin field. */
  manager?: boolean;
  /** Legacy compatibility field; never consulted as an authority grant. */
  fleetFullAccess?: boolean;
  /** Legacy compatibility field; supported agents always receive the facade. */
  mcpFacade?: boolean;
  /** Legacy compatibility field; supported agents always receive operator tools. */
  toolScope?: RemoteTokenScope;
  /** Legacy compatibility list; every enabled plugin is ambient. */
  pluginTools?: string[];
  label?: string;
  parentSessionId?: string;
  cols?: number;
  rows?: number;
  /**
   * Library item ids (kind 'mcp') selected for this spawn. Resolved to a
   * session-scoped `--mcp-config` with `--strict-mcp-config` + pre-allowed
   * tools. Ignored for facade sessions (they take the facade config).
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
   * The routing labels this dispatch arrived with (role / capability /
   * decisionId), recorded on the session so the snapshot can report them —
   * `respawn_with` inherits role + capability from there, and the hub's
   * decision log joins on decisionId. Metadata only: the ceiling clamp that
   * acts on `capability` already ran in the hub router. Omitted for an
   * unrouted spawn. See ClaudeSessionState.routing.
   */
  routing?: SessionRouting;
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
  managerReplacementState.assertResume(opts.resumeSessionId);
  return managerReplacementState.admitted([opts.parentSessionId], () => spawnClaude(opts));
}
async function spawnClaude(opts: ClaudeSpawnOptions): Promise<string> {
  if (opts.scrubProfileBypass && opts.launchIntegrationId)
    throw new Error('Launch integrations currently require a local desktop session');
  // A Claude PTY spawn takes CLAUDE profiles only. The picker filters on it,
  // but this path is also reachable from the bus, and a Codex profile applied
  // here would put a Codex config root in CLAUDE_CONFIG_DIR — a session that
  // boots into first-run onboarding and looks like a broken login.
  const pickedProfile = opts.profileId ? claudeProfiles.getProfile(opts.profileId) : undefined;
  if (pickedProfile && !profileAppliesTo(pickedProfile, 'claude')) {
    console.warn(
      `[claudeSpawn] ignoring profile '${pickedProfile.name}' — it configures ` +
        `${pickedProfile.provider}, not claude`,
    );
  }
  const rawProfile = profileAppliesTo(pickedProfile, 'claude') ? pickedProfile : undefined;
  // Profiles are user-authored harness configuration. Once an authenticated
  // agent is allowed to spawn, Workspacer does not second-guess or rewrite the
  // selected profile's config root, argv, or provider permission mode.
  const profile = rawProfile;
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
  const skipPermissions = !!opts.skipPermissions;
  // An explicit mode wins; the legacy boolean maps to bypass. Recorded on the
  // snapshot so the composer pill shows truth.
  const permissionMode = opts.permissionMode ?? (skipPermissions ? 'bypassPermissions' : 'default');
  // Whether this process will carry `--dangerously-skip-permissions` — the same
  // three inputs buildClaudeArgv resolves it from below. Recorded because Claude
  // gates *switching to* bypassPermissions on the flag, so it's what tells the
  // composer whether "Full access" is a live switch or a restart.
  const bypassAvailable =
    skipPermissions ||
    permissionMode === 'bypassPermissions' ||
    (profile?.extraArgs ?? []).includes('--dangerously-skip-permissions');
  // Reasoning effort. An explicit request wins; otherwise a MANAGER spawn takes
  // the level configured for it on this harness (Settings → Fleet Manager).
  // Resolved here — not at the renderer entry point — so it lands on every way
  // the manager starts, the same rule the model above follows, and so the
  // recorded spawn meta below names the level the argv actually carries.
  let effort = opts.effort;
  if (!effort?.trim() && opts.manager && !opts.resumeSessionId)
    effort = resolveManagerEffort('claude');

  const profileModel = modelFromExtraArgs(profile?.extraArgs);
  let requestedModel = profileModel ?? opts.model;
  if (opts.manager && !opts.resumeSessionId && !requestedModel)
    requestedModel = resolveManagerModel('claude');
  const requestedContextWindow = opts.manager
    ? resolveManagerContextForSpawn('claude', opts.contextWindow, opts.resumeSessionId)
    : opts.contextWindow;
  // A pre-feature manager card may carry no recorded selection at all. On
  // resume, omission means "let the existing conversation keep its durable
  // provider choice", not "apply today's global Claude default".
  const preserveManagerResumeSelection =
    opts.manager &&
    !!opts.resumeSessionId &&
    !requestedModel?.trim() &&
    !opts.modelIdentity?.trim();
  const modelSelection = preserveManagerResumeSelection
    ? undefined
    : resolveSpawnModelSelection(
        'claude',
        requestedModel,
        profileModel ? undefined : opts.modelIdentity,
        profileModel ? undefined : requestedContextWindow,
      );
  const model = modelSelection?.model;
  const serializedModel = modelSelection ? claudeArgvModel(modelSelection) : undefined;

  // Per-spawn MCP servers selected from the Library (kind 'mcp'). Resolve the
  // chosen item ids to their configs, write a session-scoped --mcp-config, and
  // pre-allow their tools. `--strict-mcp-config` so the session sees exactly
  // these servers, not the user's global ones. Sessions with the workspacer MCP
  // facade take the facade config instead of the user's library MCP servers.
  // Every supported Workspacer-launched agent gets the authenticated operator
  // facade. Legacy mcpFacade/toolScope/pluginTools inputs remain parseable but
  // cannot narrow (or widen) this ambient capability.
  const wantsFacade = true;
  const facadeScope: RemoteTokenScope = 'operator';
  let userMcpServers: SessionMcpServer[] = [];
  if (opts.mcpItemIds && opts.mcpItemIds.length) {
    const wanted = new Set(opts.mcpItemIds);
    // listWithSecrets, not list(): the config written below is what the CLI
    // actually authenticates with, and list() masks MCP env/headers. The real
    // values never leave main — the renderer sent only `mcpItemIds`.
    userMcpServers = libraryService
      .listWithSecrets(opts.cwd)
      .filter((it) => it.kind === 'mcp' && it.mcp && wanted.has(it.id))
      .map((it) => ({ id: it.id, mcp: it.mcp! }));
  }

  // The Fleet Manager's own coordinator model. Resolved per HARNESS
  // (lib/roleModels) rather than read straight off one flat field:
  // `agents.managerProvider` shipped with no model twin at all, so the manager
  // always ran on its harness's default with no way to choose. Resolved HERE
  // rather than at the renderer entry point so it lands on every way a manager
  // starts — the Overview hero, the palette, a respawn of a stopped card, a
  // headless bus spawn — instead of only the one that remembered to read it.
  // Then the general default, so an omitted model is RESOLVED rather than left
  // to Claude Code's own internal choice. It goes on the argv AND (below) into
  // the spawn payload, which is what lets the daemon know this session's window
  // from token zero instead of guessing 200k off a marker-stripped transcript
  // id. See lib/spawnModel.
  // The Fleet Manager's invocable skills (/bearings, /stow) — parity with the
  // stream path (managedSpawn), where the manager normally runs.
  if (opts.manager) {
    installManagerSkills();
  }
  const cardCwd = normalizeSpawnCwd(opts.cwd);
  assertSpawnCwd(cardCwd);
  // The app starts the facade asynchronously at boot. Agent launch is the hard
  // boundary: never mint a session token or inject a URL until the exact
  // facade has a connected hub and its initial plugin catalog.
  await ensureMcpFacadeReady();
  const cardInstruction = installResponseCardSkill('claude', cardCwd);
  const collaborationInstruction = installAgentCollaborationSkills(
    'claude',
    cardCwd,
    !!opts.manager,
  );

  // The facade fragment is built BEFORE the argv so the structured-result
  // contract can be appended to its --append-system-prompt instead of racing it
  // for the single flag: buildClaudeArgv takes one appendSystemPrompt, and a
  // second spread would silently drop whichever key lost. A non-facade worker
  // (the common ship-task dispatch) gets the contract as its only appended
  // prompt.
  let facadeTokenMinted = false;
  const facadeToken = mintSessionFacadeToken(
    sessionId,
    'operator',
    ['*'],
    undefined,
    undefined,
    opts.manager ? 'manager' : undefined,
  ).token;
  facadeTokenMinted = true;
  try {
    const facadeArgs =
      wantsFacade &&
      facadeSpawnArgs({
        sessionId,
        additionalServers: userMcpServers,
        // The token identifies and revokes this session. Operator authority and
        // enabled-plugin tools are ambient; legacy profile/yolo grant fields are
        // intentionally absent.
        token: facadeToken,
      });
    const resultContract = resultSchema ? buildResultContract(resultSchema) : '';
    const escalationContract = isFleetDispatchedWorker(opts) ? buildWorkerEscalationContract() : '';
    const appendSystemPrompt = [
      facadeArgs ? facadeArgs.appendSystemPrompt : '',
      escalationContract,
      resultContract,
      cardInstruction,
      collaborationInstruction,
    ]
      .filter(Boolean)
      .join('\n\n');

    const argv = buildClaudeArgv({
      extraArgs: profile?.extraArgs,
      resumeSessionId: opts.resumeSessionId,
      model,
      contextWindow: modelSelection?.contextWindow,
      effort,
      settingsFile: claudeSettingsOverlayEnabled() ? claudemonOverlayPath() : undefined,
      skipPermissions,
      permissionMode: permissionMode as 'default' | 'acceptEdits' | 'plan' | 'bypassPermissions',
      sessionId,
      // Facade sessions get the MCP config + pre-allowed tools + a role prompt.
      // The per-session bearer identifies this lifecycle and is revoked when it
      // ends; supported agents receive the ambient operator/plugin surface.
      // Built above so the structured-result contract
      // can share the one --append-system-prompt.
      ...(facadeArgs && {
        mcpConfig: facadeArgs.mcpConfig,
        allowedTools: facadeArgs.allowedTools,
        ...(userMcpServers.length && { strictMcpConfig: true }),
      }),
      ...(appendSystemPrompt && { appendSystemPrompt }),
    });
    // The cwd is used exactly as written — normalizeSpawnCwd trims and nothing
    // more, deliberately (BINDING DECISION 1:
    // no layer on a caller's path expands '~'). Which is why the pre-flight below
    // has to exist: a path that cannot be a working directory must fail HERE,
    // where the user is told, rather than as a session claudemon registers and
    // then stops the instant the child fails to launch.
    let cwd = normalizeSpawnCwd(opts.cwd);
    assertSpawnCwd(cwd);
    // A profile spawn inherits the primary login's trust for this folder, or a
    // PTY parks on the invisible trust dialog (mode "unknown", dead pane).
    if (env.CLAUDE_CONFIG_DIR) syncAccountTrust(env.CLAUDE_CONFIG_DIR, cwd);
    const prepared = await prepareLaunchIntegration(
      opts.launchIntegrationId,
      { agent: 'claude', cwd, model, resume: !!opts.resumeSessionId },
      { env, args: argv.slice(1) },
    );
    // Record name/parent before the session registers so adopted cards are
    // enriched from the very first hook event.
    claudeSessionStore.setSpawnMeta(sessionId, {
      cwd,
      label: opts.label,
      parentSessionId: opts.parentSessionId,
      isWakeTarget: opts.manager,
      provider: 'claude',
      ...(resultSchema && { resultSchema }),
      ...(opts.routing && { routing: opts.routing }),
      settings: {
        model: serializedModel,
        // Requested/provisional. Provider telemetry later owns
        // resolvedContextWindow and may correct this without rewriting history.
        contextWindow: modelSelection?.contextWindow,
        effort,
        permissionMode,
        bypassAvailable,
        // What an absent `--effort` resolves to, so the pill can name the level
        // instead of the word "Default". The CLI reports it nowhere.
        ...(!effort?.trim() && {
          defaultEffort: resolveClaudeDefaultEffort(opts.cwd, profile?.configDir),
        }),
      },
    });

    const launched = await claudemonSessionClient.spawn({
      argv: [argv[0], ...prepared.args],
      cwd,
      cols: opts.cols,
      rows: opts.rows,
      env: prepared.env,
      sessionId,
      // Explicitly, not only via `--model` on the argv: a resume re-uses the
      // prior life's model without re-stating it, and the daemon's argv sniffing
      // would find nothing to record for exactly the sessions that have the most
      // history to mis-measure.
      model: serializedModel,
      modelIdentity: modelSelection?.model,
      contextWindow: modelSelection?.contextWindow,
      firstMessage: opts.firstMessage,
    });
    facadeTokenMinted = false; // the session store owns revocation from here
    return launched;
  } catch (error) {
    if (facadeTokenMinted) {
      try {
        revokeSessionFacadeTokens(sessionId);
      } catch (cleanupError) {
        console.error(
          `[claudeSpawn] failed to revoke token for failed spawn ${sessionId}:`,
          cleanupError,
        );
      }
    }
    throw error;
  }
}
