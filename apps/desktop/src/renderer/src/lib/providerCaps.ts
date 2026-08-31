/**
 * Per-provider capability descriptor for the session controls (composer pills
 * + spawn dialog) — the minimal slice of the "Seam A" provider descriptor from
 * docs/multi-agent-providers.md. One place says what each backend can do, so
 * the composer and the spawn dialog can't drift.
 *
 * Switch semantics:
 *  - 'live'    — applied to the running session (claude: claudemon submits `/model`
 *                through the normal message path, indistinguishable from typing
 *                it).
 *  - 'restart' — the session is respawned with the new setting. Claude resumes
 *                the same conversation (`--resume` on the pinned id); managed
 *                providers (codex/opencode) start a fresh provider-side thread,
 *                which the confirm copy must say.
 */

import type { AgentProvider } from '../types/pane';

/** How a Claude session runs: the classic PTY TUI, or the headless
 *  stream-json managed adapter (GUI only, no PTY). */
export type ClaudeTransport = 'pty' | 'stream';

export interface EffortLevel {
  id: string;
  label: string;
}

const EFFORT_LABELS: Record<string, string> = {
  none: 'None',
  minimal: 'Minimal',
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'Extra high',
  max: 'Max',
  // Codex's live ladder carries these beyond the shared set; without a label the
  // menu would show the raw id.
  ultra: 'Ultra',
};

/** Display label for an effort id reported by a harness/model catalog. */
export function effortLevelLabel(id: string): string {
  return EFFORT_LABELS[id] ?? id;
}

export interface PermissionModeOption {
  id: string;
  label: string;
}

/**
 * What a harness can tell us about work it delegates or plans — the three
 * Inspector sections that are NOT universal.
 *
 * These exist because an always-present tab is a promise. "No plan yet" reads
 * as "not yet" to a user, so a harness with no plan tooling at all displays as
 * one that simply hasn't got round to it; the tab never fills in and there is
 * nothing anywhere that says why. Each flag below is the answer to "can this
 * ever be non-empty", and a `false` removes the tab rather than leaving it to
 * lie quietly.
 *
 * Every value is pinned to a verified fact about the harness — see the
 * per-provider comments, and `providerCaps.delegation.test.ts` for what each
 * claim rests on.
 */
export interface DelegationCaps {
  /** The harness has plan/todo tooling whose list reaches `session.plan`. */
  plan: boolean;
  /** The harness dispatches subagents AND our adapter turns them into
   *  `session.subagents` rows. */
  subagents: boolean;
  /** A subagent row can be opened on its own live transcript. Strictly
   *  narrower than `subagents`: a provider can report that a child ran without
   *  our having any way to read what it did. */
  subagentDrillIn: boolean;
  /** The harness runs multi-agent workflow scripts we can watch
   *  (`session.workflows`). Claude-only today — the runs are read off Claude
   *  Code's own on-disk run artifacts, which no other harness writes. */
  workflows: boolean;
}

export interface ProviderCaps {
  /** How a model change is applied mid-session. */
  modelSwitch: 'live' | 'restart';
  /** Which model list feeds the picker: claude aliases+seen vs the daemon's
   *  live `/providers/:p/models` query. */
  modelSource: 'claude' | 'managed';
  /** Reasoning-effort levels, or null when the provider has no such knob.
   *  'live' providers still fall back to the restart confirm when the switch
   *  can't be applied to the running session (a busy claude, a codex rollout
   *  fallback). */
  effort: { levels: EffortLevel[]; switch: 'live' | 'restart' } | null;
  permissionModes: PermissionModeOption[];
  /** How a permission-mode change is applied mid-session. 'live' providers
   *  still fall back to the restart confirm when the daemon reports the
   *  switch can't be done live (busy, not in the shift+tab cycle, or a
   *  bypass-spawned codex that can't re-enable approvals). */
  permissionSwitch: 'live' | 'restart';
  /** Whether a restart re-opens the same conversation (drives confirm copy). */
  restartPreservesConversation: boolean;
  /** Plan / subagents / workflows — see {@link DelegationCaps}. */
  delegation: DelegationCaps;
}

const MANAGED_PERMISSION_MODES: PermissionModeOption[] = [
  { id: 'ask', label: 'Ask to approve' },
  { id: 'yolo', label: 'Full access' },
];

// Claude Code reasoning-effort ladder (`claude --effort <level>`). Claude owns
// this harness-wide vocabulary, including `max`. Codex is different: its exact
// ladder comes from each `model/list` row at runtime (the list below is only a
// fallback while that catalog loads).
//
// Deliberately NOT the `/effort` command's full vocabulary, which also takes
// `ultracode` and `auto`: those are accepted by the slash command but not by the
// `--effort` launch flag (verified — `claude --help` lists low..max only), so a
// session left on one of them could not be reproduced by the next restart. The
// ladder stays the intersection, which is what makes it safe to carry a live
// level through a later respawn.
const CLAUDE_EFFORT_LEVELS: EffortLevel[] = [
  { id: 'low', label: 'Low' },
  { id: 'medium', label: 'Medium' },
  { id: 'high', label: 'High' },
  { id: 'xhigh', label: 'Extra high' },
  { id: 'max', label: 'Max' },
];

export const PROVIDER_CAPS: Record<AgentProvider, ProviderCaps> = {
  claude: {
    modelSwitch: 'live',
    modelSource: 'claude',
    // Live via the `/effort <level>` slash command through the message path —
    // the same mechanism as `/model`, verified against the CLI ("Set effort
    // level to high (this session only)"). Nothing reports the *effective*
    // effort back, so the pill shows what it asked for.
    effort: { levels: CLAUDE_EFFORT_LEVELS, switch: 'live' },
    permissionModes: [
      { id: 'default', label: 'Ask to approve' },
      { id: 'acceptEdits', label: 'Accept edits' },
      { id: 'plan', label: 'Plan mode' },
      { id: 'bypassPermissions', label: 'Full access' },
    ],
    // Live via claudemon's verified shift+tab cycle (`/permission-mode`).
    permissionSwitch: 'live',
    restartPreservesConversation: true,
    // The reference implementation: all three, and the only harness that has
    // workflows at all. TodoWrite → `session.plan`; SubagentStart/Stop hooks +
    // the on-disk `subagents/agent-<id>.jsonl` tail → rows you can open;
    // `workflows/wf_<runId>.json` → run cards.
    delegation: { plan: true, subagents: true, subagentDrillIn: true, workflows: true },
  },
  codex: {
    // Live via claudemon's `/sessions/:id/model` → `thread/settings/update` on
    // the running thread (app-server ws path). The rollout fallback can't do
    // it — the daemon answers 409 and the pill falls back to the restart flow.
    modelSwitch: 'live',
    modelSource: 'managed',
    // Live via the daemon's `/sessions/:id/model` → `thread/settings/update` on
    // the running thread, confirmed back by `thread/settings/updated`.
    effort: {
      levels: [
        { id: 'low', label: 'Low' },
        { id: 'medium', label: 'Medium' },
        { id: 'high', label: 'High' },
        { id: 'xhigh', label: 'Extra high' },
      ],
      switch: 'live',
    },
    permissionModes: MANAGED_PERMISSION_MODES,
    // Live via the adapter's approval flag (ask→yolo always; yolo→ask only
    // when codex wasn't spawned in bypass mode — the daemon reports which).
    permissionSwitch: 'live',
    restartPreservesConversation: false,
    // Plan: codex's native `update_plan` → `AgentUpdate::Plan`. Subagents: the
    // collab/`spawnAgent` items and child threads (`parentThreadId`), which the
    // adapter turns into rows. Drill-in works because a codex child thread is a
    // durable rollout file under `$CODEX_HOME/sessions` that the daemon replays
    // (`GET /sessions/:id/subagents/:agent_id/conversation`). No workflows.
    delegation: { plan: true, subagents: true, subagentDrillIn: true, workflows: false },
  },
  copilot: {
    // Live, and for a reason no other provider has: claudemon runs ONE
    // `copilot -p` process per TURN (`--session-id <uuid>` both creates and
    // resumes, so the conversation survives across processes). The next turn is
    // a whole new argv, so a model or effort switch simply changes what it is
    // launched with — nothing has to be applied to a running process.
    modelSwitch: 'live',
    modelSource: 'managed',
    // Copilot's own ladder (`copilot --effort`, v1.0.81) — the full seven, a
    // superset of Claude's five. Applied to the next turn, same as the model.
    effort: {
      levels: [
        { id: 'none', label: 'None' },
        { id: 'minimal', label: 'Minimal' },
        { id: 'low', label: 'Low' },
        { id: 'medium', label: 'Medium' },
        { id: 'high', label: 'High' },
        { id: 'xhigh', label: 'Extra high' },
        { id: 'max', label: 'Max' },
      ],
      switch: 'live',
    },
    // The ids are the shared managed pair (the whole bypass chain — the bus
    // clamp, the brain, the facade — speaks ask/yolo), but the LABELS are
    // copilot's own, because "Ask to approve" would be a lie here.
    //
    // Verified against the CLI: in non-interactive `-p` mode Copilot cannot ask
    // the user anything — a blocked tool comes back "Permission denied and could
    // not request permission from user" — and its BUILT-IN tools run
    // automatically whether or not `--allow-all-tools` is passed. What the allow
    // flags actually change for those is path/URL confinement: with none, reads
    // and writes outside the session's cwd are refused; `--allow-all` lifts
    // that. So those are the two tiers on offer, and the pill names them for
    // what they are.
    //
    // THIRD-PARTY MCP TOOLS ARE THE EXCEPTION and do not follow that rule: they
    // are deny-by-default at BOTH tiers, so the workspacer facade would be
    // unusable on an `ask` session. `providers/copilot.rs` grants exactly the
    // servers it registered with `--allow-tool <server>`, which is why a
    // non-yolo copilot session (a facade worker, or a Fleet Manager without
    // full access) can call facade tools while still being path-confined.
    permissionModes: [
      { id: 'ask', label: 'Workspace only' },
      { id: 'yolo', label: 'Full access' },
    ],
    // Live: the adapter reads the flag when it builds each turn's argv, so
    // both directions take effect on the next message (nothing was baked into a
    // long-lived process, which is why yolo→ask works too).
    permissionSwitch: 'live',
    // TRUE, and honestly so — the only managed provider that can say it.
    // `copilot --session-id <uuid>` resumes an existing session, so a restart
    // that reuses the workspacer session id rejoins the same conversation
    // (verified live: a codeword set in turn 1 was recalled after the process
    // had exited and a new one was launched).
    restartPreservesConversation: true,
    // Plan: copilot keeps its todo list in the session's OWN SQLite db
    // (`~/.copilot/session-state/<id>/session.db`, table `todos`), not on the
    // wire — the adapter reads it when `session.todos_changed` pings. Subagents:
    // `subagent.started` / `subagent.completed` frames carry a stable `agentId`
    // (verified live, testdata/copilot-subagent-capture.jsonl).
    //
    // Drill-in is FALSE, and that is the honest reading rather than a shrug: a
    // copilot child's frames do ride the parent's stdout stream, but nothing
    // persists them and the adapter deliberately drops them so they can't leak
    // into the parent's conversation. There is no transcript to open, so the
    // row must not offer to open one.
    delegation: { plan: true, subagents: true, subagentDrillIn: false, workflows: false },
  },
  opencode: {
    // Live: `opencode serve` applies the model per message, so claudemon's
    // `/sessions/:id/model` just restamps subsequent turns (and sets it
    // session-wide so the attached TUI agrees). Every OpenCode session drives
    // its turns this way, so there's no fallback path — it's always live.
    modelSwitch: 'live',
    modelSource: 'managed',
    effort: null,
    permissionModes: MANAGED_PERMISSION_MODES,
    // Live via the adapter's approval flag: it mediates every `permission.updated`
    // event, so ask↔yolo both flip without a restart (opencode is never spawned
    // in a bypass mode, so yolo→ask works too).
    permissionSwitch: 'live',
    restartPreservesConversation: false,
    // Plan: the `todowrite` tool (registered as `tool/todowrite` in the shipped
    // binary, v1.18.25) carries `{ todos: [{ content, status }] }` — the exact
    // shape `plan_from_value` reads. Subagents: the `task` tool, whose call
    // carries `subagent_type` + `description`; the child runs as its own
    // opencode session. Both are wired in the adapter.
    //
    // Drill-in is FALSE: the child IS a real opencode session with its own
    // events, but the adapter only counts it, and there is no read path from a
    // row to that child's messages. Costed as a follow-up, not guessed at here.
    delegation: { plan: true, subagents: true, subagentDrillIn: false, workflows: false },
  },
  pi: {
    // Restart, deliberately: the default (non-supervisor) Pi session is the
    // hybrid TUI, which has no programmatic channel to switch model or approvals
    // mid-session — so the daemon 409s and the pill falls back to a restart.
    // (Pi's RPC mode — supervisors only — *does* support `set_model` and live
    // approval mediation, and claudemon wires both; but that path isn't what the
    // composer drives, so the per-provider signal stays 'restart'.)
    modelSwitch: 'restart',
    modelSource: 'managed',
    effort: null,
    permissionModes: MANAGED_PERMISSION_MODES,
    permissionSwitch: 'restart',
    // Pi relaunches with the same `--session-id`, which *may* pick its session
    // file back up — unverified, so the copy promises the safer thing.
    restartPreservesConversation: false,
    // All three off, and this is a fact about the harness rather than a gap in
    // our adapter: pi's entire built-in tool set is bash, edit, find, grep, ls,
    // powershell, read, write (`dist/core/tools/`, v0.84.3 — and its own
    // `--help` banner says "read, bash, edit, write tools"). There is no todo
    // tool to build a plan from and no task tool to dispatch anything, so a
    // Plan or Agents tab on a pi session could never be anything but empty.
    //
    // The one caveat, and the reason this is a capability and not an assertion:
    // a pi EXTENSION can register arbitrary tools. If one ever registers a todo
    // list, this flag is the single place that has to change.
    delegation: { plan: false, subagents: false, subagentDrillIn: false, workflows: false },
  },
};

/**
 * Claude on the 'stream' transport (headless stream-json managed adapter).
 * Same permission-mode vocabulary as PTY Claude, but there is no TUI to type
 * `/model` into or shift+tab through — switches go through claudemon's
 * structural endpoints (`/sessions/:id/model`, `/permission-mode`), which the
 * adapter serves over the SDK control protocol. Both are marked 'live': when
 * the daemon can't apply one live it answers non-ok and the pill falls back to
 * the restart confirm (the standard degradation path, same as codex).
 */
const CLAUDE_STREAM_CAPS: ProviderCaps = {
  modelSwitch: 'live',
  modelSource: 'claude',
  // `/effort` is a CLI command, not a TUI one — it works on this transport too
  // (verified: the stream answers it as a command, not as a literal prompt).
  effort: { levels: CLAUDE_EFFORT_LEVELS, switch: 'live' },
  permissionModes: PROVIDER_CAPS.claude.permissionModes,
  permissionSwitch: 'live',
  // Restart resumes the same pinned session id (`--resume`), like PTY Claude.
  restartPreservesConversation: true,
  // Unchanged from PTY claude, and deliberately so: all three arrive OUT OF
  // BAND of the transport. Plan, subagent rows and workflow runs are read from
  // Claude Code's hooks and its on-disk transcript/run artifacts, which the
  // stream adapter writes exactly like the TUI does — none of it rides the
  // stream-json wire, so swapping the transport cannot take them away.
  delegation: PROVIDER_CAPS.claude.delegation,
};

export function capsFor(
  provider: AgentProvider | undefined,
  transport?: ClaudeTransport,
): ProviderCaps {
  if ((provider ?? 'claude') === 'claude' && transport === 'stream') return CLAUDE_STREAM_CAPS;
  return PROVIDER_CAPS[provider ?? 'claude'] ?? PROVIDER_CAPS.claude;
}

/** Labels for mode ids that can show up in live telemetry (hook
 *  `permission_mode`) but aren't offered in the spawn/restart menu. */
const EXTRA_MODE_LABELS: Record<string, string> = {
  auto: 'Auto',
  dontAsk: "Don't ask",
};

/**
 * What the mode pill says about a session whose permission mode NOTHING has
 * reported — no live telemetry, no recorded launch.
 *
 * It used to say "Ask to approve", because an absent id fell through to the
 * provider's first mode. That is a guess wearing the clothes of a fact, and on
 * the remote path it was reliably the WRONG guess: a headless node's sparse row
 * carried no permission fields at all, so a session running with permissions
 * bypassed displayed as the safest mode there is. A permission indicator that
 * invents a default is worse than one that admits ignorance — the user acts on
 * what it says.
 */
export const UNKNOWN_PERMISSION_LABEL = 'Unknown';

/** Display label for a permission-mode id ('acceptEdits' → 'Accept edits').
 *  An absent id is UNKNOWN, never the provider's default — see above. */
export function permissionModeLabel(
  provider: AgentProvider | undefined,
  id: string | undefined,
): string {
  if (!id) return UNKNOWN_PERMISSION_LABEL;
  const caps = capsFor(provider);
  return caps.permissionModes.find((m) => m.id === id)?.label ?? EXTRA_MODE_LABELS[id] ?? id;
}

/**
 * The mode id a session LAUNCHES with, spelled in its provider's own
 * vocabulary: Claude keeps its four modes, every managed provider has only
 * ask/yolo.
 *
 * TWIN of the two places that resolve the same thing where the session is
 * actually started — main/services/claudeSpawn.ts + managedSpawn.ts for a local
 * spawn, and the brain's `launchPermissionMode` (cmd/brain/handlers.go) for a
 * headless one. This copy exists because the WEB backend learns the answer from
 * the spawn result's `fullAccess` boolean and has to spell it as the id the
 * pill reads, without a main process to ask.
 */
export function launchPermissionMode(
  provider: AgentProvider | undefined,
  fullAccess: boolean,
  requested?: string,
): string {
  if ((provider ?? 'claude') === 'claude') {
    if (fullAccess) return 'bypassPermissions';
    // `fullAccess: false` OVERRULES the request. This is the refused-escalation
    // case: the caller asked for bypassPermissions and the hub clamped it, so
    // echoing the request back would reprint the exact lie being fixed — the
    // pill would read "Full access" for a session running with approvals on.
    return requested && !BYPASS_MODE_IDS.has(requested) ? requested : 'default';
  }
  return fullAccess ? 'yolo' : 'ask';
}

/** The spellings of "approvals off" across the providers. TWIN:
 *  main/lib/permissionBypass.ts's CONFIG_BYPASS_PERMISSION_MODES, and
 *  permissionModeMeansBypass in the brain + the MCP facade. */
const BYPASS_MODE_IDS: ReadonlySet<string> = new Set(['bypassPermissions', 'yolo']);
