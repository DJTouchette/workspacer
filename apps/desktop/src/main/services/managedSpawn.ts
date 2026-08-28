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
import { agentSkillsRoot } from '../lib/agentSkills';
import { installManagerSkills } from './managerSkills';
import { notifySystem } from './systemNotice';
import { assertSpawnCwd, normalizeSpawnCwd } from '../lib/spawnCwd';
import { explainUnsupportedManagedOptions } from '../lib/managedSpawnOptions';
import { resolveSpawnModel } from '../lib/spawnModel';
import { resolveTransport, type AgentTransport } from '../lib/spawnTransport';
import { resolveSupervisorModel } from '../lib/supervisorModel';
import {
  resolveManagerModel,
  resolveSummarizerModel,
  resolveSupervisorEffort,
  resolveManagerEffort,
} from '../lib/roleModels';

/** Install hints surfaced when a provider CLI isn't on PATH. */
const INSTALL_HINT: Record<AgentProvider, string> = {
  claude: 'Install Claude Code and make sure `claude` is on your PATH.',
  codex: 'Install the Codex CLI and make sure `codex` is on your PATH.',
  copilot: 'Install the GitHub Copilot CLI and make sure `copilot` is on your PATH.',
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
   *  runs headless (GUI-only, daemon-owned thread, no native TUI PTY), 'pty'
   *  runs the hybrid (native TUI + GUI on one thread).
   *
   *  OMITTED IS NOT "hybrid" — it is "the caller did not say", and it resolves
   *  through lib/spawnTransport (config `codex.transport`, shipped 'stream').
   *  A caller that means the hybrid has to spell 'pty', or the configured
   *  default would silently override it. */
  transport?: AgentTransport;
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
  // THE choke point for the transport default. Every managed spawn — the
  // `claude:spawn` IPC, the `agents.spawn` hub capability, a respawn, a job, a
  // supervisor/manager spawn — arrives here, so resolving it once is what makes
  // "codex is headless unless you say otherwise" a property of the app rather
  // than of whichever caller remembered to fill the field in. An explicit
  // request still wins; see lib/spawnTransport.
  const transport = resolveTransport(provider, opts.transport);
  // Codex's two session shapes and where each one runs:
  //  - 'stream' (the default): the app-server JSON-RPC adapter drives a
  //    daemon-owned thread with NO PTY at all — the exact twin of Claude's
  //    stream transport, and the shape this app is built around.
  //  - 'pty' on macOS/Linux: the same app-server adapter, plus the native TUI
  //    in a PTY rejoined onto that live thread over `--remote ws://…`, so
  //    claudemon owns both surfaces of one session (see providers/codex.rs).
  //  - 'pty' on Windows: the older rollout-tail hybrid — codex's own TUI in a
  //    PTY with claudemon tailing the rollout transcript for the GUI. The
  //    `--remote` rejoin was never verified there, so the hybrid stays on this
  //    path on Windows.
  //
  // Windows used to be pinned to the rollout hybrid UNCONDITIONALLY, which is
  // what made `transport: 'stream'` a warning-and-a-downgrade there. The ws
  // app-server was chosen precisely because plain-TCP ws works on Windows
  // (codex.rs:9-11), and headless spawns no PTY at all — so none of the ConPTY
  // concerns behind that pin apply to it. If the app-server does not come up,
  // claudemon degrades the session to the rollout hybrid IN PLACE, loudly (a
  // ⚠️ notice in the conversation + the transport stamp reset to 'pty' so the
  // pane grows its Term view back) rather than leaving a dead pane.
  if (provider === 'codex' && transport === 'pty' && process.platform === 'win32') {
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
  // Read off the RESOLVED transport, not the request: this is what the wire
  // payload and the spawn meta both key on, so a defaulted spawn and an
  // explicitly-headless one are indistinguishable from here down.
  const isCodexStream = provider === 'codex' && transport === 'stream';
  // The model this spawn is actually asking for. An omitted one is RESOLVED
  // from config here rather than left to the CLI's own internal default,
  // because the daemon can only record what it is told — and what it records is
  // the only carrier of a `[1m]` choice until the provider reports a window,
  // which on the stream transport is a whole turn away. This is the path most
  // dispatched workers take (`agents.spawn` over the bus names no model), so it
  // is where the spawn-time signal was being lost for most of the fleet.
  // Supervisors with no explicit model take the configured coordinator model
  // for THIS harness (lib/supervisorModel) — parity with the PTY Claude path,
  // which has always done this. Without it `supervisor.model` was silently
  // Claude-only: picking a codex supervisor model in Settings changed nothing.
  // The Fleet Manager takes `agents.managerModels[provider]` the same way — and
  // this is the path it actually spawns on (chat-first `transport: 'stream'`),
  // so a manager model that never reached here would be a picker writing config
  // nobody reads.
  const spawnModel = resolveSpawnModel(
    provider,
    opts.model?.trim() ||
      (opts.supervisor ? resolveSupervisorModel(provider) : undefined) ||
      (opts.manager ? resolveManagerModel(provider) : undefined),
  );
  // Reasoning effort, same rule as the model above: an explicit request wins,
  // otherwise a ROLE spawn takes the level configured for it on THIS harness
  // (supervisor.efforts / agents.managerEfforts). Per-harness because the
  // ladders are not portable — codex's 'xhigh' means nothing to claude — and
  // resolved here so it reaches every entry point, not just a launcher.
  const spawnEffort =
    opts.effort?.trim() ||
    (opts.supervisor ? resolveSupervisorEffort(provider) : undefined) ||
    (opts.manager ? resolveManagerEffort(provider) : undefined);
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
    // What the CARD believes before the daemon's first frame arrives. Codex
    // states both shapes (never just 'stream'-or-absent): 'pty' is now a real
    // choice a caller can have made, and an absent key would read as "unknown"
    // to a pane deciding whether to grow a Term view.
    ...(isClaudeStream && { transport: 'stream' as const }),
    ...(provider === 'codex' && { transport }),
    settings: {
      model: spawnModel,
      effort: spawnEffort,
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
        ...(!spawnEffort?.trim() && {
          defaultEffort: resolveClaudeDefaultEffort(cwd, profile?.configDir),
        }),
      }),
    },
  });
  const instructions = [
    wantsFacade
      ? managedFacadeInstructions({
          supervisor: !!opts.supervisor,
          scope: facadeScope,
          sessionId: managedId,
          fullAccess: supervisorFullAccess,
          // The loop parameters the PTY twin (facadeSpawnArgs) has always
          // passed — a managed supervisor was told neither, so it invented its
          // own cadence and picked its own digest-worker model.
          // Resolved for the harness the supervisor is running on, and that
          // harness is NAMED — this is the path a codex supervisor takes, and
          // it is exactly where its digest workers were falling back to Claude
          // (spawn_agent with no provider spawns Claude). See roleModels.
          summarizerProvider: provider,
          summarizerModel: resolveSummarizerModel(provider),
          pollSeconds: configService.getConfig().supervisor?.pollSeconds,
          // Only harnesses with a personal-skills directory actually got the
          // /supervise install above; telling the others to run it would send
          // them looking for a slash command that does not exist.
          superviseSkill: agentSkillsRoot(provider) !== null,
        })
      : '',
    resultSchema ? buildResultContract(resultSchema) : '',
  ]
    .filter(Boolean)
    .join('\n\n');
  const sessionId = await claudemonSessionClient.spawnManaged({
    provider,
    cwd,
    model: spawnModel,
    effort: spawnEffort,
    bin,
    yolo,
    sessionId: managedId,
    // STATED, not implied. The daemon reads an absent key as "hybrid", which is
    // the same thing a dropped field looks like — so a codex spawn always says
    // which of its two shapes it is, and a wire capture is enough to tell a
    // defaulted headless spawn from a downgraded one.
    ...(provider === 'codex' && { transport }),
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
  // Same resolution as the managed path: record what actually runs, not a hole.
  // Codex carries no configured default of its own, so this is opts.model
  // trimmed today — the call is here so a future codex.defaultModel lands on
  // both spawn paths at once instead of one.
  const spawnModel = resolveSpawnModel(
    'codex',
    opts.model?.trim() ||
      (opts.supervisor ? resolveSupervisorModel('codex') : undefined) ||
      (opts.manager ? resolveManagerModel('codex') : undefined),
  );
  // …and the same for effort: a role spawn with none requested takes the level
  // configured for it on codex (supervisor.efforts / agents.managerEfforts).
  const spawnEffort =
    opts.effort?.trim() ||
    (opts.supervisor ? resolveSupervisorEffort('codex') : undefined) ||
    (opts.manager ? resolveManagerEffort('codex') : undefined);
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
    // This branch IS a PTY session (codex's own TUI + a transcript tailer), so
    // it says so rather than leaving the field absent: with codex defaulting to
    // headless, "no transport recorded" would read as the default, not as this.
    transport: 'pty' as const,
    settings: {
      model: spawnModel,
      effort: spawnEffort,
      permissionMode: skipPermissions ? 'yolo' : 'ask',
    },
  });
  // Show the card immediately; the rollout tailer + conversation stream enrich it.
  claudeSessionStore.ensureManagedSession(sessionId, cwd);
  // Codex takes model/effort overrides as config flags (`-c model="<id>"`,
  // `-c model_reasoning_effort=<level>`); YOLO maps to bypassing its
  // approval/sandbox prompts so the TUI doesn't block on them.
  const model = spawnModel;
  const effort = spawnEffort;
  const argv = [
    bin,
    ...(model ? ['-c', `model=${JSON.stringify(model)}`] : []),
    ...(effort ? ['-c', `model_reasoning_effort=${JSON.stringify(effort)}`] : []),
    ...(skipPermissions ? ['--dangerously-bypass-approvals-and-sandbox'] : []),
  ];
  await claudemonSessionClient.spawn({
    argv,
    cwd,
    // Explicit, not sniffed off the argv: the daemon records the requested
    // model from this field, and a Codex resume puts nothing on the argv.
    model: spawnModel,
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
