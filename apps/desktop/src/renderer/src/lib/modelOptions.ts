/**
 * The model catalog behind every model picker (composer pills, handoff dialog).
 *
 * Two sources, one shape: Claude's own list (`claudeListModels` — curated
 * aliases plus concrete ids seen in past sessions) and the daemon's live
 * per-provider catalog (`providerListModels`, which boots that provider's
 * CLI/server to ask it). Kept here rather than in a component so a second
 * picker can't quietly grow a different list — the alias/seen dedupe and the
 * 1M-context marking are subtle enough to be worth having in one place.
 */

import type { AgentProvider } from '../types/pane';
import { shortModelLabel } from './modelLabel';

export interface ModelOption {
  id: string;
  label: string;
  /** Context-window badge ('200K' | '1M'). Claude list only. */
  context?: string;
  /** True for concrete ids observed in sessions (grouped after the aliases). */
  seen?: boolean;
  /** Provider-reported default model. */
  default?: boolean;
  /** Exact effort ids supported by this model, when the provider reports them. */
  effortLevels?: string[];
  /** Level this model runs at with no effort override (Codex reports it). */
  defaultEffort?: string;
}

/**
 * Load the pickable models for a provider. Never throws: a failed lookup (no
 * CLI, no auth, provider down) resolves to an empty list, which every caller
 * renders as "provider default only" or a free-text field.
 */
export async function loadModelOptions(
  provider: AgentProvider | undefined,
  modelSource: 'claude' | 'managed',
  cwd?: string,
): Promise<ModelOption[]> {
  try {
    if (modelSource === 'claude') {
      const res = await window.electronAPI.claudeListModels();
      // Date-stamped variants of one model shorten to the same label — keep the
      // first so the menu never shows two identical rows.
      const seen: ModelOption[] = [];
      for (const id of res.seen ?? []) {
        if (res.aliases.some((a) => a.value === id)) continue;
        const label = shortModelLabel(id) || id;
        if (seen.some((s) => s.label === label)) continue;
        // Fable / Mythos are 1M-native (the max is also the default), so they
        // read 1M without the `[1m]` marker that gates 1M on Opus/Sonnet.
        const is1m = id.includes('[1m]') || /fable|mythos/i.test(id);
        seen.push({ id, label, context: is1m ? '1M' : '200K', seen: true });
      }
      return [
        ...res.aliases.map((a) => ({ id: a.value, label: a.label, context: a.context })),
        ...seen,
      ];
    }
    const res = await window.electronAPI.providerListModels(
      provider as 'codex' | 'opencode' | 'pi',
      cwd,
    );
    return res.map((m) => ({
      id: m.id,
      label: m.label || m.id,
      default: m.default,
      effortLevels: m.effortLevels,
      defaultEffort: m.defaultEffort,
    }));
  } catch {
    return [];
  }
}
