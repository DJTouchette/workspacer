/**
 * The `claude:spawn` request shape, and its TOTAL translation into a managed
 * (Tier-2) provider spawn.
 *
 * Why this exists as its own module rather than an object literal inside the
 * IPC handler: the literal silently dropped `manager` and `fleetFullAccess` for
 * every non-Claude provider. Both are load-bearing for the Fleet Manager — no
 * `manager` means the session is never marked `isWakeTarget`, so NO
 * worker-finished wake is ever routed to it (claudeSessionStore.nudgeParentOnFinish
 * requires `parent.isWakeTarget`), and no `fleetFullAccess` means its facade
 * token is minted without the profilesAllowed / yolo grants, so every worker it
 * dispatches is clamped and prompts on everything. A Fleet Manager on Codex was
 * therefore impossible, and the failure was invisible: the card came up looking
 * exactly like a working manager.
 *
 * The fix is not "remember to add the field". It is to make forgetting a
 * COMPILE error and dropping a LOGGED one:
 *
 *   - `SPAWN_REQUEST_FIELDS` is `satisfies Record<keyof AgentSpawnRequest, …>`,
 *     so a new request field fails to compile until it is classified.
 *   - Every field is classified as 'forward' (reaches the managed spawn),
 *     'derived' (folded into another option — e.g. permissionMode → yolo),
 *     'conditional' (reaches SOME harnesses, with the reason the rest can't
 *     take it), or 'unsupported' WITH A REASON. Both reason strings are what
 *     spawnManagedAgent logs when a caller actually set the field (see
 *     explainUnsupportedManagedOptions).
 *
 * TWIN: the Claude PTY / claude-stream branches of the same IPC handler pass
 * their own fields directly; they carry everything, so they need no table.
 */
import type { RemoteTokenScope } from '../shared/ipcTypes';
import type { ManagedSpawnOptions } from '../services/managedSpawn';
import { permissionModeMeansBypass } from './permissionBypass';
import { providerTakesProfiles } from '../shared/agentProfiles';

/** The options the `claude:spawn` IPC accepts (every provider/transport). */
export interface AgentSpawnRequest {
  cwd?: string;
  provider?: 'claude' | 'codex' | 'copilot' | 'opencode' | 'pi';
  /** Claude: 'pty' (classic TUI) or 'stream' (headless stream-json, managed
   *  adapter). Codex: 'stream' runs headless (no native TUI PTY), 'pty' the
   *  hybrid (native TUI + GUI on one app-server thread).
   *  Omitted = the harness's config default (claude.transport /
   *  codex.transport) — see main/lib/spawnTransport. */
  transport?: 'pty' | 'stream';
  /** A profile of the CHOSEN harness: its config root, extra argv and native
   *  preset. Carried by claude, codex and copilot; OpenCode and Pi have no
   *  config-root override, so it is announced and dropped for them. */
  profileId?: string;
  /** Fleet Manager: nudge-eligible parent without the /supervise loop. */
  manager?: boolean;
  /** Manager only: full-access dispatch grant (config agents.fleetFullAccess) —
   *  its workers run with permissions bypassed. */
  fleetFullAccess?: boolean;
  model?: string;
  /** Canonical suffix-free identity paired with contextWindow. `model` remains
   *  the marker-bearing compatibility spelling for old main/peer versions. */
  modelIdentity?: string;
  contextWindow?: number | null;
  effort?: string;
  permissionMode?: string;
  skipPermissions?: boolean;
  resumeSessionId?: string;
  cols?: number;
  rows?: number;
  mcpFacade?: boolean;
  /** Facade tool tier: 'view' | 'triage' | 'operator' (implies the facade). */
  toolScope?: RemoteTokenScope;
  /** Plugin ids whose contributed facade tools the session may use. */
  pluginTools?: string[];
  label?: string;
  parentSessionId?: string;
  mcpItemIds?: string[];
  /** Federation: spawn on this peer hub instead of locally (the spawn dialog's
   *  Machine picker). */
  targetHub?: string;
  /** The agent's first prompt, delivered BY THE SPAWN (claudemon queues it
   *  before answering) instead of a second call once the id comes back. See
   *  ManagedSpawnOptions.firstMessage for the window that closes. */
  message?: string;
}

/**
 * What happens to each request field on the MANAGED (non-Claude-PTY) path.
 * `satisfies Record<keyof AgentSpawnRequest, …>` is the guard: adding a field
 * to AgentSpawnRequest without classifying it here is a type error.
 */
export const SPAWN_REQUEST_FIELDS = {
  cwd: { kind: 'forward' },
  provider: { kind: 'forward' },
  transport: { kind: 'forward' },
  // WAS 'unsupported: Claude accounts have no equivalent on this provider',
  // which was never true and is now false in the code as well as in the prose.
  // A profile is a CONFIG ROOT, and three harnesses have one — CLAUDE_CONFIG_DIR,
  // CODEX_HOME, COPILOT_HOME (shared/agentProfiles PROFILE_CAPS). The two that
  // genuinely don't are OpenCode and Pi, and that is what the reason now says.
  profileId: {
    kind: 'conditional',
    on: 'harnesses with a config root — claude, codex, copilot',
    why: 'OpenCode and Pi have no config-root override, so a profile picked for them would set nothing',
  },
  manager: { kind: 'forward' },
  fleetFullAccess: { kind: 'forward' },
  model: { kind: 'forward' },
  modelIdentity: { kind: 'forward' },
  contextWindow: { kind: 'forward' },
  effort: { kind: 'forward' },
  permissionMode: { kind: 'derived', into: 'skipPermissions' },
  skipPermissions: { kind: 'forward' },
  resumeSessionId: { kind: 'forward' },
  cols: { kind: 'forward' },
  rows: { kind: 'forward' },
  mcpFacade: { kind: 'forward' },
  toolScope: { kind: 'forward' },
  pluginTools: { kind: 'forward' },
  label: { kind: 'forward' },
  parentSessionId: { kind: 'forward' },
  mcpItemIds: {
    kind: 'unsupported',
    why: 'Library MCP selections ride Claude’s --mcp-config; managed providers register servers their own way',
  },
  targetHub: { kind: 'derived', into: 'a federated hub:<peer>/agents.spawn before dispatch' },
  message: { kind: 'derived', into: 'firstMessage (the daemon queues it as the first prompt)' },
} satisfies Record<
  keyof AgentSpawnRequest,
  | { kind: 'forward' }
  | { kind: 'derived'; into: string }
  // Carried to the harnesses named in `on`, announced to the rest.
  | { kind: 'conditional'; on: string; why: string }
  | { kind: 'unsupported'; why: string }
>;

/** Permission modes a managed provider expresses natively (ask/yolo pair). */
const MANAGED_PERMISSION_MODES: ReadonlySet<string> = new Set(['ask', 'yolo', 'default']);

/**
 * Translate a spawn request into managed-provider spawn options. TOTAL over
 * SPAWN_REQUEST_FIELDS: everything marked 'forward' is here, so a role flag can
 * no longer be lost between the two.
 *
 * `provider` is passed separately because the caller has already resolved the
 * default ('claude') and dispatched the Claude branches away.
 */
export function managedOptionsFromRequest(
  provider: NonNullable<AgentSpawnRequest['provider']>,
  req: AgentSpawnRequest,
): ManagedSpawnOptions {
  if (req.transport && provider !== 'codex') {
    console.warn(
      `[managedSpawn] ${provider}: ignoring transport '${req.transport}' — ` +
        'only Claude and Codex have more than one session shape',
    );
  }
  return {
    provider,
    cwd: req.cwd,
    // Codex mirrors Claude's two transports: 'stream' spawns headless (GUI-only,
    // daemon-owned thread — the default), 'pty' the hybrid. BOTH values are
    // forwarded, and that is the whole point: an omitted key now means "resolve
    // the configured default" inside spawnManagedAgent, so forwarding only
    // 'stream' (as this did) would have turned an explicit hybrid request into a
    // headless spawn the moment the default flipped. No other managed adapter
    // accepts a transport at all, so the key stays OFF their payload — but the
    // request is ANNOUNCED above rather than vanishing, which is this module's rule.
    ...(provider === 'codex' && req.transport && { transport: req.transport }),
    model: req.model,
    modelIdentity: req.modelIdentity,
    contextWindow: req.contextWindow,
    effort: req.effort,
    // Managed providers have only ask/yolo, so an explicit bypass mode folds
    // into the boolean instead of being dropped. permissionModeMeansBypass is
    // the shared spelling table ('bypassPermissions' and 'yolo' both count) —
    // matching only 'yolo' here used to leave a Claude-spelled bypass unapplied.
    skipPermissions: !!req.skipPermissions || permissionModeMeansBypass(req.permissionMode),
    permissionMode: req.permissionMode,
    resumeSessionId: req.resumeSessionId,
    // The Fleet Manager flag. Dropping it was the bug this module exists for:
    // no `manager` = no isWakeTarget = no worker-finished wake ever routed here.
    manager: req.manager,
    fleetFullAccess: req.fleetFullAccess,
    mcpFacade: req.mcpFacade,
    toolScope: req.toolScope,
    pluginTools: req.pluginTools,
    label: req.label,
    parentSessionId: req.parentSessionId,
    cols: req.cols,
    rows: req.rows,
    // profileId is APPLIED by spawnManagedAgent for every harness with a config
    // root (claude/codex/copilot) and announced for the two without one.
    // mcpItemIds stays Claude-only and is carried so the spawn can SAY it is
    // ignoring it rather than dropping it behind a guard.
    profileId: req.profileId,
    mcpItemIds: req.mcpItemIds,
    // The request calls it `message` (the vocabulary a dispatcher uses); the
    // spawn helper calls it `firstMessage` (what it is to the session). One
    // rename, here, so neither surface has to carry the other's word.
    firstMessage: req.message,
  };
}

/**
 * The human-readable list of options this managed spawn cannot honour, given
 * what the caller actually set. Empty when nothing was requested that the
 * provider can't carry. spawnManagedAgent logs it — the standing rule on this
 * path is that an option a provider cannot carry is ANNOUNCED, never quietly
 * omitted, because a silently-degraded agent looks identical to a working one.
 */
export function explainUnsupportedManagedOptions(opts: ManagedSpawnOptions): string[] {
  const out: string[] = [];
  // Claude-stream is a managed provider too, and it carries everything.
  if (opts.provider === 'claude') return out;
  // Only the harnesses that genuinely cannot take one. A codex/copilot spawn
  // now APPLIES its profile (config root + argv +, for copilot, the referenced
  // token), so warning here would have been the mirror image of the old bug:
  // announcing a drop that no longer happens.
  if (opts.profileId && !providerTakesProfiles(opts.provider)) {
    out.push(`profileId — ${SPAWN_REQUEST_FIELDS.profileId.why}`);
  }
  if (opts.mcpItemIds?.length) out.push(`mcpItemIds — ${SPAWN_REQUEST_FIELDS.mcpItemIds.why}`);
  if (
    opts.permissionMode &&
    !MANAGED_PERMISSION_MODES.has(opts.permissionMode) &&
    !permissionModeMeansBypass(opts.permissionMode)
  ) {
    out.push(
      `permissionMode '${opts.permissionMode}' — managed providers only have ask/yolo (spawning in ask)`,
    );
  }
  if (opts.provider === 'pi' && (opts.mcpFacade || opts.toolScope)) {
    out.push('the workspacer MCP facade — pi ships no MCP client, so its tools cannot attach');
  }
  if (opts.provider === 'copilot') {
    // Copilot's capability surface is the only DYNAMIC one in the fleet: the
    // CLI takes MCP servers as a flag (a better seam than any other provider),
    // but a GitHub org policy can disable third-party MCP servers entirely, and
    // when it does the CLI reports zero servers and carries on working. So we
    // cannot say at spawn time whether the facade will attach — only warn that
    // it might not. The adapter checks `session.mcp_servers_loaded` at runtime
    // and raises a session error if it didn't (providers/copilot.rs).
    if (opts.mcpFacade || opts.toolScope) {
      out.push(
        'the workspacer MCP facade — copilot supports MCP, but a GitHub org policy can disable third-party servers; if it is on, this agent starts with no workspacer tools and says so in its pane',
      );
    }
    // The mode ids are ask/yolo like every managed provider, but ask does NOT
    // mean approvals here — see providerCaps.ts. Say so at spawn, once, rather
    // than letting the pill imply a gate that does not exist.
    if (!opts.skipPermissions) {
      out.push(
        "approval prompts — copilot's non-interactive mode cannot ask, so tools run automatically; 'ask' confines them to the session's directory instead",
      );
    }
  }
  return out;
}
