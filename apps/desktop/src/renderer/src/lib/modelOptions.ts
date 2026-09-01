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
import {
  claudeAliasSelection,
  claudeArgvModel,
  formatClaudeAliasWindow,
  isClaudeInherentOneMillionModel,
  modelSelectionKey,
  normalizeModelSelection,
  sameModelSelection,
  type ModelSelection,
} from '../../../main/shared/modelContextWindows';

export interface ModelOption {
  /** Stable picker identity: the canonical model/window pair. */
  key: string;
  /** Provider-native model identity. For Claude this is never window-decorated. */
  id: string;
  /** Selectable context window, independent from identity. */
  contextWindow?: number | null;
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
  /** Installed harness catalog metadata; informational, not confirmation. */
  defaultContextWindow?: number;
  maxContextWindow?: number;
  effectiveContextWindowPercent?: number;
}

interface ClaudeCatalogPayload {
  aliases?: Array<{
    model?: string;
    /** Legacy desktop/hub payload field. */
    value?: string;
    label: string;
    contextWindow?: number | null;
    context?: string;
  }>;
  seen?: string[];
}

/** Normalize both generations of the catalog wire shape without emitting an
 * undefined command, and keep same-model/different-window rows distinct. */
export function claudeCatalogOptions(res: ClaudeCatalogPayload): ModelOption[] {
  const aliases: ModelOption[] = [];
  for (const alias of res.aliases ?? []) {
    const raw = alias.model ?? alias.value;
    if (!raw) continue;
    try {
      let selection = normalizeModelSelection(raw, alias.contextWindow);
      // Transitional payloads put the legacy marker only in `value` while the
      // new `model` field is canonical. Recover that window when necessary.
      if (selection.contextWindow === null && alias.value) {
        const legacy = normalizeModelSelection(alias.value);
        if (legacy.model.toLowerCase() === selection.model.toLowerCase()) selection = legacy;
      }
      if (selection.contextWindow === null) selection = claudeAliasSelection(selection.model);
      aliases.push({
        key: modelSelectionKey(selection),
        id: selection.model,
        contextWindow: selection.contextWindow,
        label: alias.label,
        context: alias.context ?? formatClaudeAliasWindow(claudeArgvModel(selection)),
      });
    } catch {
      // A malformed compatibility row is omitted; it must never become an
      // option whose command is the string "undefined".
    }
  }

  const seen: ModelOption[] = [];
  for (const raw of res.seen ?? []) {
    try {
      const selection = normalizeModelSelection(raw);
      if (
        aliases.some((a) =>
          sameModelSelection({ model: a.id, contextWindow: a.contextWindow ?? null }, selection),
        )
      )
        continue;
      const label = shortModelLabel(selection.model) || selection.model;
      if (seen.some((s) => s.label === label)) continue;
      seen.push({
        key: modelSelectionKey(selection),
        id: selection.model,
        label,
        contextWindow: selection.contextWindow,
        context: formatClaudeAliasWindow(raw),
        seen: true,
      });
    } catch {
      // Ignore malformed history entries.
    }
  }
  return [...aliases, ...seen];
}

export function modelOptionCommand(option: ModelOption): string {
  return claudeArgvModel({ model: option.id, contextWindow: option.contextWindow ?? null });
}

/** Match live state as a normalized pair. Display names may be concrete Claude
 * ids, so family matching is retained, but suffix presence alone is not. */
export function modelOptionMatches(
  option: ModelOption,
  currentModel: string | null | undefined,
  currentWindow?: number | null,
): boolean {
  if (!currentModel) return false;
  let current: ModelSelection;
  try {
    current = normalizeModelSelection(currentModel, currentWindow);
    // A bare current alias has no wire window. Choose the contract's base row
    // deterministically (Opus/Sonnet 200K; inherent Fable/Mythos 1M) instead
    // of matching no row or making the explicit 1M sibling ambiguous.
    if (current.contextWindow === null) current = claudeAliasSelection(current.model);
  } catch {
    return false;
  }
  const picked = normalizeModelSelection(option.id, option.contextWindow);
  const family = (model: string) =>
    (shortModelLabel(model).match(/^[A-Za-z]+/)?.[0] ?? model).toLowerCase();
  return (
    family(picked.model) === family(current.model) && picked.contextWindow === current.contextWindow
  );
}

/** The model half of App's remembered Claude spawn defaults. Omission means
 * this spawn did not make a model choice (prompt-first/palette), so the patch
 * must not contain either key. An explicit picker value is canonicalized as a
 * pair before config.yaml sees it. */
export function rememberedClaudeModelPatch(model: string | undefined): {
  defaultModel?: string;
  contextWindow?: number | null;
} {
  if (model === undefined) return {};
  if (!model.trim()) return { defaultModel: '', contextWindow: null };
  const selection = normalizeModelSelection(model);
  if (selection.contextWindow === null && isClaudeInherentOneMillionModel(selection.model)) {
    selection.contextWindow = 1_000_000;
  }
  return { defaultModel: selection.model, contextWindow: selection.contextWindow };
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
      return claudeCatalogOptions(res);
    }
    const res = await window.electronAPI.providerListModels(
      provider as 'codex' | 'copilot' | 'opencode' | 'pi',
      cwd,
    );
    return res.map((m) => ({
      key: modelSelectionKey({ model: m.id, contextWindow: null }),
      id: m.id,
      label: m.label || m.id,
      default: m.default,
      effortLevels: m.effortLevels,
      defaultEffort: m.defaultEffort,
      defaultContextWindow: m.defaultContextWindow,
      maxContextWindow: m.maxContextWindow,
      effectiveContextWindowPercent: m.effectiveContextWindowPercent,
    }));
  } catch {
    return [];
  }
}
