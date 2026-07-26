/**
 * Per-provider capability descriptor for the session controls (composer pills
 * + spawn dialog) — the minimal slice of the "Seam A" provider descriptor from
 * docs/multi-agent-providers.md. One place says what each backend can do, so
 * the composer and the spawn dialog can't drift.
 *
 * Switch semantics:
 *  - 'live'    — applied to the running session (claude: `/model` is submitted
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

/** Display label for a permission-mode id ('acceptEdits' → 'Accept edits'). */
export function permissionModeLabel(
  provider: AgentProvider | undefined,
  id: string | undefined,
): string {
  const caps = capsFor(provider);
  const fallback = caps.permissionModes[0];
  if (!id) return fallback?.label ?? 'Ask to approve';
  return caps.permissionModes.find((m) => m.id === id)?.label ?? EXTRA_MODE_LABELS[id] ?? id;
}
