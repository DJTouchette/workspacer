/**
 * Single source of truth for the model picker data, shared by the IPC handler
 * (desktop) and the hub capability (web). Aliases resolve to the latest model of
 * each family so they track Claude Code updates with zero maintenance; `seen`
 * carries concrete ids observed in real transcripts — persisted in config plus
 * anything live in the session store.
 */

import { configService } from './configService';
import {
  claudeAliasSelection,
  claudeArgvModel,
  formatContextWindow,
  ModelSelectionError,
  normalizeModelSelection,
} from '../shared/modelContextWindows';
import { claudeSessionStore } from './claudeSessionStore';

export interface ClaudeModelAlias {
  /** Canonical identity. */
  model: string;
  /** Legacy picker adapter; canonical identity for old value-only clients. */
  value: string;
  label: string;
  contextWindow: number;
  /** Context-window badge, e.g. '200K' | '1M'. */
  context?: string;
}

export interface ListModelsResult {
  defaultModel: string;
  contextWindow: number | null;
  skipPermissionsDefault: boolean;
  /** Permission mode remembered from the last spawn ('' = provider default). */
  defaultPermissionMode: string;
  aliases: ClaudeModelAlias[];
  seen: string[];
}

/** Family + dotted version from a concrete id, e.g.
 *  "claude-opus-4-8-20250101" → { family: 'opus', version: '4.8' }. */
function parseConcreteId(id: string): { family: string; version: string } | null {
  let identity: string;
  try {
    identity = normalizeModelSelection(id).model;
  } catch {
    return null;
  }
  const m = identity.match(/^claude-([a-z]+)-(\d+(?:-\d+)*?)(?:-\d{6,})?$/);
  if (!m) return null;
  return { family: m[1], version: m[2].replace(/-/g, '.') };
}

function newerVersion(a: string, b: string): boolean {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d > 0;
  }
  return false;
}

export function listClaudeModels(): ListModelsResult {
  const cfg = configService.getConfig() as any;
  const persisted: string[] = Array.isArray(cfg.claude?.seenModels) ? cfg.claude.seenModels : [];
  const live = claudeSessionStore
    .getAllSnapshots()
    // Federation: remote sessions run on another machine's account/install —
    // their model ids don't belong in the local picker's "seen" list.
    .filter((s) => !s.hub)
    .map((s) => s.usage?.model)
    .filter((m): m is string => !!m);
  // `<synthetic>` is Claude Code's placeholder model id on synthetic transcript
  // messages — telemetry noise, not a launchable model.
  const seenAll = Array.from(
    new Set(
      [...persisted, ...live].flatMap((id) => {
        try {
          return [normalizeModelSelection(id).model];
        } catch {
          return [];
        }
      }),
    ),
  )
    .filter((id) => !id.startsWith('<'))
    .sort();

  // Newest concrete version observed per family (opus → '4.8'), used to
  // version-label the alias rows below.
  const newest = new Map<string, string>();
  for (const id of seenAll) {
    const p = parseConcreteId(id);
    if (p && (!newest.has(p.family) || newerVersion(p.version, newest.get(p.family)!)))
      newest.set(p.family, p.version);
  }

  // An alias already stands for the newest model of its family, so a seen id at
  // that same version would render as a duplicate row — absorb it into the
  // alias (which carries its version in the label) and keep only older ids.
  const seen = seenAll.filter((id) => {
    const p = parseConcreteId(id);
    return !p || newest.get(p.family) !== p.version;
  });

  const label = (family: string, base: string) =>
    newest.has(family) ? `${base} ${newest.get(family)}` : base;

  const rawDefault = typeof cfg.claude?.defaultModel === 'string' ? cfg.claude.defaultModel : '';
  let defaultSelection = { model: '', contextWindow: null as number | null };
  if (rawDefault.trim()) {
    try {
      defaultSelection = normalizeModelSelection(rawDefault, cfg.claude?.contextWindow);
    } catch (err) {
      if (!(err instanceof ModelSelectionError)) throw err;
      // Config loading already reports and isolates an invalid configured
      // selection. Keep the catalog usable if a mixed-version/remote answer
      // nevertheless carries one; the Go twin returns the same empty default.
    }
  }
  const alias = (legacyValue: string, aliasLabel: string): ClaudeModelAlias => {
    const selection = claudeAliasSelection(legacyValue);
    if (selection.contextWindow === null) {
      throw new Error(`Claude alias ${legacyValue} has no context-window contract`);
    }
    return {
      model: selection.model,
      // Old clients know only `value`, so it must remain an executable Claude
      // selection. New clients use canonical `model` + explicit window.
      value: claudeArgvModel(selection),
      label: aliasLabel,
      contextWindow: selection.contextWindow,
      context: formatContextWindow(selection.contextWindow),
    };
  };

  return {
    defaultModel: defaultSelection.model,
    contextWindow: defaultSelection.contextWindow,
    skipPermissionsDefault: cfg.claude?.skipPermissionsDefault === true,
    defaultPermissionMode:
      typeof cfg.claude?.defaultPermissionMode === 'string' ? cfg.claude.defaultPermissionMode : '',
    // Each alias tracks the newest model of its family, so these need zero
    // maintenance as Claude Code updates. `sonnet[1m]` is Claude Code's own
    // alias for the million-token-context Sonnet.
    // The `context` badge is LOOKED UP, never spelled here: these strings were
    // a display-only sixth window table, unpinned to the four numeric ones, and
    // `formatClaudeAliasWindow` reads the same contract every other window in
    // the repo now comes from.
    aliases: [
      // Fable's 1M window is both its maximum AND its default — there is no 200K
      // mode to select, so (unlike Opus/Sonnet) it has no separate `[1m]` row and
      // always shows 1M.
      alias('fable', label('fable', 'Fable')),
      alias('opus', label('opus', 'Opus')),
      alias('opus[1m]', label('opus', 'Opus')),
      alias('sonnet', label('sonnet', 'Sonnet')),
      alias('sonnet[1m]', label('sonnet', 'Sonnet')),
      alias('haiku', label('haiku', 'Haiku')),
    ],
    seen,
  };
}
