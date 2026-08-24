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
import { claudeProfiles, scrubBypassProfile, scrubRemoteGrantedProfile } from './claudeProfiles';
import { syncAccountTrust } from './claudeAccountSetup';
import { resolveClaudeDefaultEffort } from './claudeEffortDefault';
import { libraryService } from './libraryService';
import { resolveAgentBinary, isAgentBinaryInstalled, type AgentProvider } from './agentProviders';
import { configService } from './configService';
import {
  MCP_FACADE_URL,
  managedFacadeInstructions,
  buildSessionMcpConfig,
  facadeSessionMcpConfig,
  facadeUrlWithToken,
} from './mcpConfig';
import { mintSessionFacadeToken } from './remoteTokens';
import { managerFullAccessFromConfig } from './fullAccessGrants';
import { buildResultContract, checkResultSchema } from '../shared/structuredResult';
import type { RemoteTokenScope } from '../shared/ipcTypes';
import { claudemonOverlayPath, claudeSettingsOverlayEnabled } from './claudemonDaemon';
import { ensureSupervisorHome, installSupervisorSkill } from './supervisorSkill';
import { installManagerSkills } from './managerSkills';
import { notifySystem } from './systemNotice';
import { assertSpawnCwd, normalizeSpawnCwd } from '../lib/spawnCwd';
import { explainUnsupportedManagedOptions } from '../lib/managedSpawnOptions';

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
  /** Claude: must be 'stream' when provider === 'claude'. Codex: 'stream'
   *  runs headless (GUI-only, daemon-owned thread, no native TUI PTY) —
   *  omitted/other means the default hybrid. */
  transport?: 'stream';
  cwd?: string;
  model?: string;
  /** Reasoning-effort level (Claude `--effort`, Codex config); others ignore it. */
  effort?: string;
  /** YOLO / auto-approve every command and file change. */
  skipPermissions?: boolean;
  /** Claude (stream) only: explicit permission mode
   *  (default/acceptEdits/plan/bypassPermissions). Managed providers use the
   *  ask/yolo pair via `skipPermissions`. */
  permissionMode?: string;
  /** Strip any permission bypass the chosen PROFILE carries — set on untrusted
   *  boundaries (the hub/remote spawn capability). See scrubBypassArgs. */
  scrubProfileBypass?: boolean;
  /** The hub verified this spawn's caller holds a token whose profilesAllowed
   *  grant names this profileId (fleet-manager dispatch). Softens the scrub to
   *  scrubRemoteGrantedProfile — configDir kept, bypass args and mcpItemIds
   *  still stripped. Only hubCapabilities sets it, from the hub-STAMPED param
   *  (sanitizeSpawnParams deletes any caller-supplied copy). Meaningless
   *  without scrubProfileBypass. */
  profileGranted?: boolean;
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
  /** Fleet Manager: a nudge-eligible parent (worker finished/blocked wakes
   *  route to it) WITHOUT the /supervise loop or supervisor role text — its
   *  doctrine rides its kickoff message. Callers pair it with
   *  toolScope 'operator'. */
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
  /** PTY dimensions for hybrid (PTY-backed) providers like Codex. */
  cols?: number;
  rows?: number;
  /**
   * Structured-result contract: a JSON Schema the dispatcher wants the worker's
   * final report to carry as a fenced `wks-result` block. Rides the daemon's
   * first-turn `instructions` here (the managed twin of the PTY path's
   * --append-system-prompt) and is validated at the worker-finished wake.
   * Purely additive — the prose report is unaffected.
   */
  resultSchema?: Record<string, unknown>;
  /**
   * The agent's FIRST PROMPT — the dispatch itself — carried by the spawn
   * instead of a separate `agents.sendMessage` once the id comes back.
   *
   * DISTINCT FROM `instructions`, and it has to be. `instructions` (the facade
   * role note + the result contract, joined below) is a passive PREFIX: every
   * adapter parks it in a `pending_instructions` slot and prepends it to the
   * first prompt the session receives, so it never starts a turn on its own. A
   * dispatch put there would wait forever for the prompt it *is*. Sent as a
   * real prompt, this gets the ordering right for free — contract first, task
   * second, one turn, once.
   *
   * The window it closes is measured, not theoretical: claudemon's
   * `register_managed` marks the row `Input` with no wrapper attached, so a
   * `POST /message` issued between the spawn's 200 and the provider driver's
   * `register_managed_input` is refused with a 404. The prompt is queued inside
   * the spawn handler instead, and drained by that same registration.
   */
  firstMessage?: string;
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
  // ANNOUNCE, never silently omit. An option this provider cannot carry is
  // logged with the reason — the Fleet-Manager-on-Codex bug was a hand-copied
  // option literal quietly dropping `manager`/`fleetFullAccess`, and a
  // half-wired agent is indistinguishable from a working one until the wakes
  // that never arrive are noticed hours later. Both entry points (the
  // `claude:spawn` IPC and the `agents.spawn` hub capability) land here, so
  // this covers them at once.
  for (const why of explainUnsupportedManagedOptions(opts)) {
    console.warn(`[managedSpawn] ${provider}: ignoring ${why}`);
  }
  // Codex is a hybrid (GUI + Term) on every platform, but the wiring differs:
  //  - macOS/Linux: the app-server JSON-RPC adapter (the generic managed path
  //    below) drives the structured GUI *and* spawns the native TUI in a PTY,
  //    resumed onto the same live app-server thread over `--remote ws://…` — so
  //    claudemon owns both surfaces of one session (see providers/codex.rs).
  //  - Windows: the older rollout-tail hybrid — its own TUI runs in a PTY and
  //    claudemon tails the rollout transcript for the GUI. Kept until the ws
  //    app-server path is verified on Windows, at which point this branch (and
  //    the rollout tailer) can go and all platforms share the managed path.
  if (provider === 'codex' && process.platform === 'win32') {
    if (opts.transport === 'stream') {
      console.warn(
        '[managedSpawn] codex headless (stream) is not available on Windows yet — spawning the rollout hybrid',
      );
    }
    return spawnCodexHybrid(opts);
  }
  // Supervisors with no explicit cwd open in their dedicated home (~/.workspacer)
  // rather than inheriting some repo; everything else uses the given cwd.
  let cwd = normalizeSpawnCwd(opts.cwd);
  if (opts.supervisor && !opts.cwd) cwd = ensureSupervisorHome();
  // Refused here, before a session id exists: this path's 200 arrives BEFORE
  // the adapter's child launches, so an unusable cwd otherwise surfaces as a
  // live-looking card whose session is already stopped (see spawnCwd.ts).
  assertSpawnCwd(cwd);

  const isClaudeStream = provider === 'claude';
  // Codex's stream transport mirrors Claude's: headless, GUI-only, no PTY.
  const isCodexStream = provider === 'codex' && opts.transport === 'stream';
  const bin = resolveAgentBinary(provider, configuredBin(provider));
  const wantsFacade = opts.supervisor || opts.mcpFacade || !!opts.toolScope;
  // A supervisor is operator by definition; a plain facade session takes its
  // requested tier, defaulting to operator (the legacy mcpFacade meaning).
  const facadeScope: RemoteTokenScope = opts.supervisor
    ? 'operator'
    : (opts.toolScope ?? 'operator');
  const managedId = opts.resumeSessionId || randomUUID();
  // Refused out loud rather than dropped — see claudeSpawn's twin.
  const resultSchema = opts.resultSchema;
  if (resultSchema !== undefined) {
    const bad = checkResultSchema(resultSchema);
    if (bad) throw new Error(`spawn: ${bad}`);
  }
  // Supervisor full-access mode (config supervisor.fullAccess, the supervisor
  // twin of agents.fleetFullAccess): the supervisor itself runs bypassed, and
  // its facade token below carries the yolo grant so the workers it spawns may
  // run bypassed too. Config-resolved — not a caller flag — so every entry
  // point (IPC, hub bus, jobs) applies the local user's setting identically.
  // TWIN: claudeSpawn.ts resolves the same setting on the PTY path.
  const supervisorFullAccess =
    !!opts.supervisor && configService.getConfig().supervisor?.fullAccess === true;
  const skipPermissions = !!opts.skipPermissions || supervisorFullAccess;
  // Per-session scoped facade token. Pi ships no MCP client, so minting one
  // for it would only leave a dangling live secret.
  // A host-blessed Fleet Manager's token carries a dispatch grant for every
  // local profile — the hub verifies it and stamps profileGranted on the
  // worker spawn. Only `manager` gets this; a plain supervisor or facade
  // worker has no business spawning as other accounts.
  // The yolo grant is CONFIG-RESOLVED for both roles (never a caller flag —
  // a respawn's frozen fleetFullAccess must not resurrect a revoked grant),
  // and the role tag lets a later config flip update it LIVE. TWIN:
  // claudeSpawn.ts mints identically on the PTY path.
  const facadeToken =
    wantsFacade && provider !== 'pi'
      ? mintSessionFacadeToken(
          managedId,
          facadeScope,
          opts.pluginTools,
          opts.manager ? claudeProfiles.getProfiles().map((p) => p.id) : undefined,
          opts.manager ? managerFullAccessFromConfig() : supervisorFullAccess || undefined,
          opts.manager ? 'manager' : opts.supervisor ? 'supervisor' : undefined,
        ).token
      : undefined;
  // Permission-mode vocabulary differs by family: Claude keeps its full mode
  // set (an explicit mode wins; the legacy boolean maps to bypass, and
  // supervisor full access forces it — same resolution as the PTY path),
  // managed providers are just ask/yolo.
  const permissionMode = isClaudeStream
    ? supervisorFullAccess
      ? 'bypassPermissions'
      : (opts.permissionMode ?? (skipPermissions ? 'bypassPermissions' : 'default'))
    : skipPermissions
      ? 'yolo'
      : 'ask';
  const yolo = isClaudeStream
    ? skipPermissions || permissionMode === 'bypassPermissions'
    : skipPermissions;
  // Claude (stream) parity with the PTY path (claudeSpawn.ts): a profile maps
  // to CLAUDE_CONFIG_DIR + its extra argv, and Library MCP selections become a
  // session-scoped --mcp-config with --strict-mcp-config + pre-allowed tools.
  // Both ride the spawn-managed payload's claude-only env/extra_args fields
  // instead of being silently dropped. Facade sessions take the facade MCP
  // config instead of the user's library servers, as on the PTY path.
  const profile =
    isClaudeStream && opts.profileId
      ? opts.scrubProfileBypass
        ? opts.profileGranted
          ? scrubRemoteGrantedProfile(claudeProfiles.getProfile(opts.profileId))
          : scrubBypassProfile(claudeProfiles.getProfile(opts.profileId))
        : claudeProfiles.getProfile(opts.profileId)
      : undefined;
  const env: Record<string, string> = {};
  if (profile?.configDir) {
    env.CLAUDE_CONFIG_DIR = profile.configDir.replace(/^~/, os.homedir());
    // A profile spawn inherits the primary login's trust for this folder —
    // without it the account's own .claude.json (unlinked by design, and
    // seeded empty by the old wrong-path read) gates the spawn on a trust
    // dialog no headless/GUI surface ever renders.
    syncAccountTrust(env.CLAUDE_CONFIG_DIR, cwd);
  }
  const extraArgs: string[] = [...(profile?.extraArgs ?? [])];
  // Overlay settings (hooks + statusLine) so stream sessions carry our hooks
  // without mutating the user's global settings.json — the stream analogue of
  // the PTY path's `--settings` in buildClaudeArgv.
  if (isClaudeStream && claudeSettingsOverlayEnabled()) {
    extraArgs.push('--settings', claudemonOverlayPath());
  }
  // Transport parity (FLEET_MANAGER_SPIKE.md finding #3): the PTY path has
  // always installed the /supervise skill for supervisors; the stream path
  // never did, so a stream-transport supervisor got role text but no skill.
  // PROVIDER parity too: the install is routed to the directory THIS harness
  // reads (~/.claude/skills vs $CODEX_HOME/skills — identical SKILL.md format),
  // rather than being gated on Claude and leaving a codex manager with no
  // slash commands at all. Same doctrine text everywhere by design.
  if (opts.supervisor) {
    installSupervisorSkill(provider);
  }
  // The Fleet Manager gets its own invocable skills (/standup, /checkpoint,
  // /handoff) — the considered counterpart to its reactive brief doctrine.
  if (opts.manager) {
    installManagerSkills(provider);
  }
  // Claude stream + facade: the per-session config file (token as an
  // Authorization header — a file path on argv, never the token itself, since
  // /proc/<pid>/cmdline is world-readable). The PTY path's twin lives in
  // facadeSpawnArgs; pre-allowing mcp__workspacer matches it.
  if (isClaudeStream && facadeToken) {
    extraArgs.push('--mcp-config', facadeSessionMcpConfig(managedId, facadeToken));
    extraArgs.push('--allowedTools', 'mcp__workspacer');
  }
  if (isClaudeStream && !wantsFacade && opts.mcpItemIds && opts.mcpItemIds.length) {
    const wanted = new Set(opts.mcpItemIds);
    // listWithSecrets, not list(): the config written below is what the CLI
    // actually authenticates with, and list() masks MCP env/headers. The real
    // values never leave main — the renderer sent only `mcpItemIds`.
    const servers = libraryService
      .listWithSecrets(opts.cwd)
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
    // Managers count: the nudge router (supervisorSessionIds) is keyed on
    // this flag, and a manager IS a supervisor for wake purposes.
    isSupervisor: opts.supervisor || opts.manager,
    provider,
    ...(resultSchema && { resultSchema }),
    ...((isClaudeStream || isCodexStream) && { transport: 'stream' as const }),
    settings: {
      model: opts.model,
      effort: opts.effort,
      permissionMode,
      // Claude only: `yolo` is exactly the `--dangerously-skip-permissions` the
      // adapter puts on the argv, and Claude gates live switches *to*
      // bypassPermissions on it (the control protocol refuses otherwise). The
      // managed providers have no such mode, so the field stays absent for them.
      ...(isClaudeStream && {
        bypassAvailable: yolo || extraArgs.includes('--dangerously-skip-permissions'),
        // Claude reports its effective effort nowhere, so what an absent
        // `--effort` resolves to is read from the settings chain at spawn (the
        // stream argv takes the same flag as the PTY one). Codex's default comes
        // from its live model catalog instead — the composer reads it there.
        ...(!opts.effort?.trim() && {
          defaultEffort: resolveClaudeDefaultEffort(cwd, profile?.configDir),
        }),
      }),
    },
  });
  const instructions = [
    wantsFacade
      ? managedFacadeInstructions(!!opts.supervisor, facadeScope, managedId, supervisorFullAccess)
      : '',
    resultSchema ? buildResultContract(resultSchema) : '',
  ]
    .filter(Boolean)
    .join('\n\n');
  const sessionId = await claudemonSessionClient.spawnManaged({
    provider,
    cwd,
    model: opts.model,
    effort: opts.effort,
    bin,
    yolo,
    sessionId: managedId,
    ...(isCodexStream && { transport: 'stream' as const }),
    // Codex resume: claudemon rejoins the prior life's app-server thread and
    // replays its rollout (headless-only; the daemon forces stream transport).
    ...(provider === 'codex' && opts.resumeSessionId && { resumeSessionId: opts.resumeSessionId }),
    // Claude stream adapter extras: the full permission mode and (on a
    // respawn) the prior conversation to `--resume`.
    ...(isClaudeStream && {
      permissionMode,
      ...(opts.resumeSessionId && { resumeSessionId: opts.resumeSessionId }),
      ...(extraArgs.length && { extraArgs }),
      ...(Object.keys(env).length && { env }),
    }),
    ...(wantsFacade && {
      // Claude stream carries the facade via the --mcp-config file above, so
      // no `mcp` URL for it. Codex/OpenCode registrations are URL-only (a `-c`
      // override / opencode.json) and cannot send headers, so their token
      // rides a `?t=` query param the facade also accepts; pi has no MCP
      // client at all and keeps the bare URL no-op + role instructions.
      ...(!isClaudeStream && {
        mcp: facadeToken ? facadeUrlWithToken(facadeToken) : MCP_FACADE_URL,
      }),
    }),
    // First-turn instructions: the facade role note (when this session has the
    // facade) and the structured-result contract (when the dispatch carried a
    // schema), joined so neither overwrites the other — the daemon takes ONE
    // instructions string. A plain worker with a schema and no facade still
    // gets its contract, which is the common ship-task dispatch.
    ...(instructions && { instructions }),
    // The dispatch prompt itself — a SEPARATE field, never folded into
    // `instructions` above, because `instructions` alone never starts a turn
    // (see ManagedSpawnOptions.firstMessage). The daemon prepends one to the
    // other, so the contract still lands ahead of the task.
    ...(opts.firstMessage && { firstMessage: opts.firstMessage }),
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
  assertSpawnCwd(cwd);
  const bin = resolveAgentBinary('codex', configuredBin('codex'));
  const sessionId = opts.resumeSessionId || randomUUID();
  // Same supervisor full-access resolution as the managed path above.
  const skipPermissions =
    !!opts.skipPermissions ||
    (!!opts.supervisor && configService.getConfig().supervisor?.fullAccess === true);
  // The Windows rollout hybrid predates the facade wiring: it spawns a bare TUI
  // and tails the transcript, so a manager/facade session asked for here comes
  // up WITHOUT its tools. Said out loud rather than discovered later.
  if (opts.manager || opts.supervisor || opts.mcpFacade || opts.toolScope) {
    console.warn(
      '[managedSpawn] codex (Windows rollout hybrid): the workspacer MCP facade is not wired on this path — ' +
        'this session gets no workspacer tools (wake routing still applies)',
    );
  }
  claudeSessionStore.setSpawnMeta(sessionId, {
    label: opts.label,
    parentSessionId: opts.parentSessionId,
    // Managers are wake targets exactly like supervisors — same flag, same
    // reason as the managed path above.
    isSupervisor: opts.supervisor || opts.manager,
    provider: 'codex',
    settings: {
      model: opts.model,
      effort: opts.effort,
      permissionMode: skipPermissions ? 'yolo' : 'ask',
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
    ...(skipPermissions ? ['--dangerously-bypass-approvals-and-sandbox'] : []),
  ];
  await claudemonSessionClient.spawn({
    argv,
    cwd,
    cols: opts.cols ?? 120,
    rows: opts.rows ?? 32,
    sessionId,
    rolloutProvider: 'codex',
    // This branch spawns a bare TUI (no facade, no `instructions`), so the
    // dispatch is the only host-injected text it gets — dropping it here would
    // leave the one provider path whose worker never learns its task.
    firstMessage: opts.firstMessage,
  });
  return sessionId;
}
