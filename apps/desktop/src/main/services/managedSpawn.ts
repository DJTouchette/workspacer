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
import { claudeSessionStore, type SessionRouting } from './claudeSessionStore';
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
import {
  profileAppliesTo,
  profileConfigEnv,
  profileSpawnArgs,
  profileTokenEnv,
  providerTakesProfiles,
} from '../shared/agentProfiles';
import type { RemoteTokenScope } from '../shared/ipcTypes';
import { claudemonOverlayPath, claudeSettingsOverlayEnabled } from './claudemonDaemon';
import { installManagerSkills } from './managerSkills';
import { notifySystem } from './systemNotice';
import { assertSpawnCwd, normalizeSpawnCwd } from '../lib/spawnCwd';
import { explainUnsupportedManagedOptions } from '../lib/managedSpawnOptions';
import { resolveSpawnModel } from '../lib/spawnModel';
import { resolveTransport, type AgentTransport } from '../lib/spawnTransport';
import { resolveManagerModel, resolveManagerEffort } from '../lib/roleModels';

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
  /** A profile of THIS provider's harness: its config root (CLAUDE_CONFIG_DIR /
   *  CODEX_HOME / COPILOT_HOME) + extraArgs + any native preset — same
   *  semantics as the Claude PTY path (claudeSpawn.ts). A profile belonging to
   *  a different harness is ignored and logged: it would point the wrong root
   *  at the wrong CLI. */
  profileId?: string;
  /** Claude (stream) only: Library item ids (kind 'mcp') selected for this
   *  spawn, resolved to a session-scoped `--mcp-config` with
   *  `--strict-mcp-config` + pre-allowed tools — same as the PTY path. */
  mcpItemIds?: string[];
  /** Re-use this id (matches the desktop's pinned-session contract). */
  resumeSessionId?: string;
  /** Fleet Manager: a nudge-eligible parent (worker finished/blocked wakes
   *  route to it) — its doctrine rides its kickoff message. Callers pair it
   *  with toolScope 'operator'. */
  manager?: boolean;
  /** Manager full-access HINT from the caller. The token's actual yolo grant
   *  is config-resolved at mint (services/fullAccessGrants), so this flag no
   *  longer decides anything here; it
   *  is kept on the wire for record fidelity (the renderer persists it on the
   *  agent card and re-passes it on respawn). */
  fleetFullAccess?: boolean;
  /** Wire the facade tools at the legacy operator tier — prefer `toolScope`. */
  mcpFacade?: boolean;
  /**
   * Grant the workspacer facade tools at a TIER: 'view' (observe-only — right
   * for summarizer workers), 'triage' (view + approve/reply/interrupt), or
   * 'operator' (everything). Mints a per-session scoped token the facade
   * enforces, so the agent sees (and pays context for) only its tier's tools.
   * Implies the facade; `mcpFacade` without it means 'operator'.
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
  // manager spawn — arrives here, so resolving it once is what makes
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
  let cwd = normalizeSpawnCwd(opts.cwd);
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
  // The Fleet Manager takes `agents.managerModels[provider]` — and
  // this is the path it actually spawns on (chat-first `transport: 'stream'`),
  // so a manager model that never reached here would be a picker writing config
  // nobody reads.
  const spawnModel = resolveSpawnModel(
    provider,
    opts.model?.trim() || (opts.manager ? resolveManagerModel(provider) : undefined),
  );
  // Reasoning effort, same rule as the model above: an explicit request wins,
  // otherwise a MANAGER spawn takes the level configured for it on THIS
  // harness (agents.managerEfforts). Per-harness because the
  // ladders are not portable — codex's 'xhigh' means nothing to claude — and
  // resolved here so it reaches every entry point, not just a launcher.
  const spawnEffort =
    opts.effort?.trim() || (opts.manager ? resolveManagerEffort(provider) : undefined);
  const bin = resolveAgentBinary(provider, configuredBin(provider));
  const wantsFacade = opts.mcpFacade || !!opts.toolScope;
  // A facade session takes its requested tier, defaulting to operator (the
  // legacy mcpFacade meaning).
  const facadeScope: RemoteTokenScope = opts.toolScope ?? 'operator';
  const managedId = opts.resumeSessionId || randomUUID();
  // Refused out loud rather than dropped — see claudeSpawn's twin.
  const resultSchema = opts.resultSchema;
  if (resultSchema !== undefined) {
    const bad = checkResultSchema(resultSchema);
    if (bad) throw new Error(`spawn: ${bad}`);
  }
  const skipPermissions = !!opts.skipPermissions;
  // Per-session scoped facade token. Pi ships no MCP client, so minting one
  // for it would only leave a dangling live secret.
  // A host-blessed Fleet Manager's token carries a dispatch grant for every
  // local profile — the hub verifies it and stamps profileGranted on the
  // worker spawn. Only `manager` gets this; a plain facade worker has no
  // business spawning as other accounts.
  // The yolo grant is CONFIG-RESOLVED (never a caller flag —
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
          opts.manager ? managerFullAccessFromConfig() : undefined,
          opts.manager ? 'manager' : undefined,
        ).token
      : undefined;
  // Permission-mode vocabulary differs by family: Claude keeps its full mode
  // set (an explicit mode wins; the legacy boolean maps to bypass — same
  // resolution as the PTY path), managed providers are just ask/yolo.
  const permissionMode = isClaudeStream
    ? (opts.permissionMode ?? (skipPermissions ? 'bypassPermissions' : 'default'))
    : skipPermissions
      ? 'yolo'
      : 'ask';
  const yolo = isClaudeStream
    ? skipPermissions || permissionMode === 'bypassPermissions'
    : skipPermissions;
  // A profile maps to its harness's CONFIG ROOT plus its extra argv. The root
  // is Claude-specific only by history — every harness with profiles has one
  // (CLAUDE_CONFIG_DIR / CODEX_HOME / COPILOT_HOME, PROFILE_CAPS) — so the
  // lookup is no longer gated on Claude. Library MCP selections stay Claude's
  // alone: they become a session-scoped --mcp-config with --strict-mcp-config +
  // pre-allowed tools, and managed providers register servers their own way.
  // Facade sessions take the facade MCP config instead of the user's library
  // servers, as on the PTY path.
  //
  // profileAppliesTo re-checks the harness even though both pickers filter on
  // it: this dispatch is reachable from the hub bus, where no picker ran, and a
  // Claude profile applied to a Codex spawn would point CODEX_HOME at a Claude
  // config root — a broken session that looks like a working one.
  const picked =
    opts.profileId && providerTakesProfiles(provider)
      ? claudeProfiles.getProfile(opts.profileId)
      : undefined;
  const rawProfile = profileAppliesTo(picked, provider) ? picked : undefined;
  if (opts.profileId && picked && !rawProfile) {
    console.warn(
      `[managedSpawn] ignoring profile '${picked.name}' — it configures ` +
        `${picked.provider ?? 'claude'}, not ${provider}`,
    );
  }
  const profile = rawProfile
    ? opts.scrubProfileBypass
      ? opts.profileGranted
        ? scrubRemoteGrantedProfile(rawProfile)
        : scrubBypassProfile(rawProfile)
      : rawProfile
    : undefined;
  // The config root, plus (Copilot) the auth token the profile REFERENCES by
  // variable name — resolved from this process's environment here, at spawn,
  // and never stored. An unset name contributes nothing rather than an empty
  // token that would out-rank copilot's own stored credential.
  const env: Record<string, string> = {
    ...profileConfigEnv(profile, os.homedir()),
    ...profileTokenEnv(profile, process.env),
  };
  if (env.CLAUDE_CONFIG_DIR) {
    // A Claude profile spawn inherits the primary login's trust for this folder
    // — without it the account's own .claude.json (unlinked by design, and
    // seeded empty by the old wrong-path read) gates the spawn on a trust
    // dialog no headless/GUI surface ever renders. Claude-only because the
    // trust map is a Claude file; the other harnesses have no equivalent.
    syncAccountTrust(env.CLAUDE_CONFIG_DIR, cwd);
  }
  // extraArgs plus the harness's native preset flag (`codex -p <name>`) when
  // the profile set one — see profileSpawnArgs for why the preset is appended
  // after extraArgs.
  const extraArgs: string[] = profileSpawnArgs(profile);
  // Overlay settings (hooks + statusLine) so stream sessions carry our hooks
  // without mutating the user's global settings.json — the stream analogue of
  // the PTY path's `--settings` in buildClaudeArgv.
  if (isClaudeStream && claudeSettingsOverlayEnabled()) {
    extraArgs.push('--settings', claudemonOverlayPath());
  }
  // The Fleet Manager gets its own invocable skills (/standup, /checkpoint,
  // /handoff) — the considered counterpart to its reactive brief doctrine.
  // The install is routed to the directory THIS harness reads
  // (~/.claude/skills vs $CODEX_HOME/skills — identical SKILL.md format).
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
    // The nudge router (supervisorSessionIds) is keyed on this flag: the
    // manager is the wake target.
    isWakeTarget: opts.manager,
    provider,
    ...(resultSchema && { resultSchema }),
    ...(opts.routing && { routing: opts.routing }),
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
          scope: facadeScope,
          sessionId: managedId,
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
    }),
    // The profile's config-root env and its extra argv. Sent for EVERY harness
    // that takes profiles, not just Claude: `CODEX_HOME` / `COPILOT_HOME` are
    // the same primitive as `CLAUDE_CONFIG_DIR`, and `codex -p <preset>` rides
    // the same argv channel. Both keys stay off the payload when empty, so a
    // profile-less spawn is byte-identical to what it sent before.
    //
    // THE DAEMON HALF IS NOT DONE YET. Read at 2026-08-28 on this branch:
    // `daemon/spawn.rs` `/sessions/spawn-managed` forwards `env`/`extra_args`
    // into the `"claude"` (claude_stream) arm ONLY — `codex::spawn_session` and
    // `copilot::SpawnConfig` take neither, so for those two harnesses these
    // keys reach the daemon and stop there. Everything above (the store, the
    // scrub, the harness re-check, the token resolution) is correct and pinned
    // by managedSpawn.test.ts, but a Codex/Copilot profile does not change the
    // spawned process until the Rust side threads these two fields through.
    // Do not "fix" this by dropping the keys — the wire contract is the half
    // that is right.
    ...(extraArgs.length && { extraArgs }),
    ...(Object.keys(env).length && { env }),
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
  assertSpawnCwd(cwd);
  const bin = resolveAgentBinary('codex', configuredBin('codex'));
  const sessionId = opts.resumeSessionId || randomUUID();
  // Same resolution as the managed path: record what actually runs, not a hole.
  // Codex carries no configured default of its own, so this is opts.model
  // trimmed today — the call is here so a future codex.defaultModel lands on
  // both spawn paths at once instead of one.
  const spawnModel = resolveSpawnModel(
    'codex',
    opts.model?.trim() || (opts.manager ? resolveManagerModel('codex') : undefined),
  );
  // …and the same for effort: a manager spawn with none requested takes the
  // level configured for it on codex (agents.managerEfforts).
  const spawnEffort =
    opts.effort?.trim() || (opts.manager ? resolveManagerEffort('codex') : undefined);
  const skipPermissions = !!opts.skipPermissions;
  // The Windows rollout hybrid predates the facade wiring: it spawns a bare TUI
  // and tails the transcript, so a manager/facade session asked for here comes
  // up WITHOUT its tools. Said out loud rather than discovered later.
  if (opts.manager || opts.mcpFacade || opts.toolScope) {
    console.warn(
      '[managedSpawn] codex (Windows rollout hybrid): the workspacer MCP facade is not wired on this path — ' +
        'this session gets no workspacer tools (wake routing still applies)',
    );
  }
  claudeSessionStore.setSpawnMeta(sessionId, {
    label: opts.label,
    parentSessionId: opts.parentSessionId,
    // The manager is the wake target — same flag, same reason as the managed
    // path above.
    isWakeTarget: opts.manager,
    provider: 'codex',
    // The hybrid branch records routing too: it is reached through
    // spawnManagedAgent (codex on transport 'pty'), so the same dispatch can
    // land here, and a routed worker whose snapshot forgot its role is exactly
    // the silent loss this field exists to prevent.
    ...(opts.routing && { routing: opts.routing }),
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
