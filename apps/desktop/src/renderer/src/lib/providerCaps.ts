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
  /** Reasoning-effort levels, or null when the provider has no such knob. */
  effort: { levels: EffortLevel[]; switch: 'restart' } | null;
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

// Claude Code reasoning-effort ladder (`claude --effort <level>`). Distinct from
// codex's `minimal/low/medium/high` — Claude adds `xhigh` and `max` and has no
// `minimal`. Default is `high`; the CLI's own default is `xhigh`. Restart-only:
// there's a `--effort` flag but no `/effort` slash command to change it live.
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
    // Reasoning effort via `claude --effort <level>`, applied at spawn. There's
    // no `/effort` slash command, so a change respawns (--resume keeps the
    // conversation) — same restart flow as codex.
    effort: { levels: CLAUDE_EFFORT_LEVELS, switch: 'restart' },
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
    effort: {
      levels: [
        { id: 'minimal', label: 'Minimal' },
        { id: 'low', label: 'Low' },
        { id: 'medium', label: 'Medium' },
        { id: 'high', label: 'High' },
      ],
      switch: 'restart',
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
  // Same `--effort` flag on the headless argv; restart-to-apply as on PTY.
  effort: { levels: CLAUDE_EFFORT_LEVELS, switch: 'restart' },
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
