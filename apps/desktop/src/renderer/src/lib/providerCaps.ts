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
  /** Whether a restart re-opens the same conversation (drives confirm copy). */
  restartPreservesConversation: boolean;
}

const MANAGED_PERMISSION_MODES: PermissionModeOption[] = [
  { id: 'ask', label: 'Ask to approve' },
  { id: 'yolo', label: 'Full access' },
];

export const PROVIDER_CAPS: Record<AgentProvider, ProviderCaps> = {
  claude: {
    modelSwitch: 'live',
    modelSource: 'claude',
    // Claude Code has thinking budgets but no stable CLI/slash control for
    // them today — hidden until one exists.
    effort: null,
    permissionModes: [
      { id: 'default', label: 'Ask to approve' },
      { id: 'acceptEdits', label: 'Accept edits' },
      { id: 'plan', label: 'Plan mode' },
      { id: 'bypassPermissions', label: 'Full access' },
    ],
    restartPreservesConversation: true,
  },
  codex: {
    modelSwitch: 'restart',
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
    restartPreservesConversation: false,
  },
  opencode: {
    modelSwitch: 'restart',
    modelSource: 'managed',
    effort: null,
    permissionModes: MANAGED_PERMISSION_MODES,
    restartPreservesConversation: false,
  },
  pi: {
    modelSwitch: 'restart',
    modelSource: 'managed',
    effort: null,
    permissionModes: MANAGED_PERMISSION_MODES,
    // Pi relaunches with the same `--session-id`, which *may* pick its session
    // file back up — unverified, so the copy promises the safer thing.
    restartPreservesConversation: false,
  },
};

export function capsFor(provider: AgentProvider | undefined): ProviderCaps {
  return PROVIDER_CAPS[provider ?? 'claude'] ?? PROVIDER_CAPS.claude;
}

/** Display label for a permission-mode id ('acceptEdits' → 'Accept edits'). */
export function permissionModeLabel(provider: AgentProvider | undefined, id: string | undefined): string {
  const caps = capsFor(provider);
  const fallback = caps.permissionModes[0];
  if (!id) return fallback?.label ?? 'Ask to approve';
  return caps.permissionModes.find((m) => m.id === id)?.label ?? id;
}
