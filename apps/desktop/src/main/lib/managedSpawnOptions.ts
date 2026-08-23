/**
 * The `claude:spawn` request shape, and its TOTAL translation into a managed
 * (Tier-2) provider spawn.
 *
 * Why this exists as its own module rather than an object literal inside the
 * IPC handler: the literal silently dropped `manager` and `fleetFullAccess` for
 * every non-Claude provider. Both are load-bearing for the Fleet Manager — no
 * `manager` means the session is never marked `isSupervisor`, so NO
 * worker-finished wake is ever routed to it (claudeSessionStore.nudgeParentOnFinish
 * requires `parent.isSupervisor`), and no `fleetFullAccess` means its facade
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
 *     'derived' (folded into another option — e.g. permissionMode → yolo), or
 *     'unsupported' WITH A REASON, which spawnManagedAgent logs when a caller
 *     actually set it (see explainUnsupportedManagedOptions).
 *
 * TWIN: the Claude PTY / claude-stream branches of the same IPC handler pass
 * their own fields directly; they carry everything, so they need no table.
 */
import type { RemoteTokenScope } from '../shared/ipcTypes';
import type { ManagedSpawnOptions } from '../services/managedSpawn';
import { permissionModeMeansBypass } from './permissionBypass';

/** The options the `claude:spawn` IPC accepts (every provider/transport). */
export interface AgentSpawnRequest {
  cwd?: string;
  provider?: 'claude' | 'codex' | 'opencode' | 'pi';
  /** Claude: 'pty' (classic TUI) or 'stream' (headless stream-json, managed
   *  adapter); omitted = the config default (claude.transport).
   *  Codex: 'stream' runs headless (no native TUI PTY). */
  transport?: 'pty' | 'stream';
  profileId?: string;
  /** Fleet Manager: nudge-eligible parent without the /supervise loop. */
  manager?: boolean;
  /** Manager only: full-access dispatch grant (config agents.fleetFullAccess) —
   *  its workers run with permissions bypassed. */
  fleetFullAccess?: boolean;
  model?: string;
  effort?: string;
  permissionMode?: string;
  skipPermissions?: boolean;
  resumeSessionId?: string;
  cols?: number;
  rows?: number;
  supervisor?: boolean;
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
  profileId: {
    kind: 'unsupported',
    why: 'Claude accounts (CLAUDE_CONFIG_DIR) have no equivalent on this provider',
  },
  manager: { kind: 'forward' },
  fleetFullAccess: { kind: 'forward' },
  model: { kind: 'forward' },
  effort: { kind: 'forward' },
  permissionMode: { kind: 'derived', into: 'skipPermissions' },
  skipPermissions: { kind: 'forward' },
  resumeSessionId: { kind: 'forward' },
  cols: { kind: 'forward' },
  rows: { kind: 'forward' },
  supervisor: { kind: 'forward' },
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
} satisfies Record<
  keyof AgentSpawnRequest,
  { kind: 'forward' } | { kind: 'derived'; into: string } | { kind: 'unsupported'; why: string }
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
  if (req.transport === 'stream' && provider !== 'codex') {
    console.warn(
      `[managedSpawn] ${provider}: ignoring transport 'stream' — ` +
        'only Claude and Codex have a headless transport',
    );
  }
  return {
    provider,
    cwd: req.cwd,
    // Codex mirrors Claude's stream transport: 'stream' spawns headless
    // (GUI-only, daemon-owned thread). No other managed adapter accepts a
    // transport at all, so the key stays OFF their payload — but the request is
    // ANNOUNCED here rather than vanishing, which is this module's whole rule.
    ...(provider === 'codex' && req.transport === 'stream' && { transport: 'stream' as const }),
    model: req.model,
    effort: req.effort,
    // Managed providers have only ask/yolo, so an explicit bypass mode folds
    // into the boolean instead of being dropped. permissionModeMeansBypass is
    // the shared spelling table ('bypassPermissions' and 'yolo' both count) —
    // matching only 'yolo' here used to leave a Claude-spelled bypass unapplied.
    skipPermissions: !!req.skipPermissions || permissionModeMeansBypass(req.permissionMode),
    permissionMode: req.permissionMode,
    resumeSessionId: req.resumeSessionId,
    supervisor: req.supervisor,
    // The Fleet Manager pair. Dropping these was the bug this module exists for:
    // no `manager` = no isSupervisor = no worker-finished wake ever routed here.
    manager: req.manager,
    fleetFullAccess: req.fleetFullAccess,
    mcpFacade: req.mcpFacade,
    toolScope: req.toolScope,
    pluginTools: req.pluginTools,
    label: req.label,
    parentSessionId: req.parentSessionId,
    cols: req.cols,
    rows: req.rows,
    // Carried so spawnManagedAgent can SAY it is ignoring them rather than
    // dropping them behind an isClaudeStream guard (both are Claude-only).
    profileId: req.profileId,
    mcpItemIds: req.mcpItemIds,
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
  if (opts.profileId) out.push(`profileId — ${SPAWN_REQUEST_FIELDS.profileId.why}`);
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
  if (opts.provider === 'pi' && (opts.supervisor || opts.mcpFacade || opts.toolScope)) {
    out.push('the workspacer MCP facade — pi ships no MCP client, so its tools cannot attach');
  }
  return out;
}
