import {
  isClaudeInherentOneMillionModel,
  ModelSelectionError,
  normalizeModelSelection,
} from './modelContextWindows';
import { isForeignModel, type Harness } from './modelVocabulary';

export type ManagerContextWindows = Partial<Record<Harness, number | null>>;

export interface ManagerSelectionPreferences {
  managerModels?: Partial<Record<Harness, string>>;
  managerEfforts?: Partial<Record<Harness, string>>;
  managerContextWindows?: ManagerContextWindows;
}

export type ManagerSelectionErrorCode =
  | 'invalid-manager-map'
  | 'invalid-manager-model'
  | 'invalid-manager-effort'
  | 'invalid-context-window'
  | 'unsupported-context-window'
  | 'foreign-manager-model';

export class ManagerSelectionError extends Error {
  constructor(
    readonly code: ManagerSelectionErrorCode,
    message: string,
  ) {
    super(message);
    this.name = 'ManagerSelectionError';
  }
}

const PROVIDERS: readonly Harness[] = ['claude', 'codex', 'copilot', 'opencode', 'pi'];
const CONTEXT_PROVIDERS = new Set<Harness>(['claude', 'codex']);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function provider(value: string): Harness | undefined {
  return (PROVIDERS as readonly string[]).includes(value) ? (value as Harness) : undefined;
}

function invalid(strict: boolean, code: ManagerSelectionErrorCode, message: string): false {
  if (strict) throw new ManagerSelectionError(code, message);
  return false;
}

/**
 * Canonical manager preference boundary shared by config read/save and spawn
 * resolution. In strict mode malformed writes are refused. In compatibility
 * mode (config read) bad entries are omitted while every unrelated preference
 * survives, so one stale mixed-version key cannot invalidate the whole file.
 *
 * Claude's legacy `[1m]` model spelling is migrated into the suffix-free model
 * plus `managerContextWindows.claude`. Codex keeps `null` as a first-class
 * explicit choice: it means provider-default and is observably different from
 * an absent entry, whose fresh-spawn policy requests the shared 1M default.
 */
export function canonicalManagerPreferences(
  value: unknown,
  strict = true,
): ManagerSelectionPreferences {
  if (value == null) return {};
  if (!isRecord(value)) {
    invalid(strict, 'invalid-manager-map', 'agents must be an object');
    return {};
  }

  const models: Partial<Record<Harness, string>> = {};
  const efforts: Partial<Record<Harness, string>> = {};
  const contexts: ManagerContextWindows = {};

  const rawModels = value.managerModels;
  if (rawModels !== undefined && !isRecord(rawModels)) {
    invalid(strict, 'invalid-manager-map', 'agents.managerModels must be an object');
  } else if (isRecord(rawModels)) {
    for (const [key, raw] of Object.entries(rawModels)) {
      const harness = provider(key);
      if (!harness || typeof raw !== 'string') {
        invalid(
          strict,
          'invalid-manager-model',
          `agents.managerModels.${key} must be a string for a known provider`,
        );
        continue;
      }
      const model = raw.trim();
      if (!model) {
        models[harness] = '';
        continue;
      }
      if (isForeignModel(harness, model)) {
        invalid(
          strict,
          'foreign-manager-model',
          `agents.managerModels.${key} belongs to another provider`,
        );
        continue;
      }
      models[harness] = model;
    }
  }

  const rawEfforts = value.managerEfforts;
  if (rawEfforts !== undefined && !isRecord(rawEfforts)) {
    invalid(strict, 'invalid-manager-map', 'agents.managerEfforts must be an object');
  } else if (isRecord(rawEfforts)) {
    for (const [key, raw] of Object.entries(rawEfforts)) {
      const harness = provider(key);
      if (!harness || typeof raw !== 'string') {
        invalid(
          strict,
          'invalid-manager-effort',
          `agents.managerEfforts.${key} must be a string for a known provider`,
        );
        continue;
      }
      efforts[harness] = raw.trim();
    }
  }

  const rawContexts = value.managerContextWindows;
  if (rawContexts !== undefined && !isRecord(rawContexts)) {
    invalid(strict, 'invalid-manager-map', 'agents.managerContextWindows must be an object');
  } else if (isRecord(rawContexts)) {
    for (const [key, raw] of Object.entries(rawContexts)) {
      const harness = provider(key);
      if (!harness || !CONTEXT_PROVIDERS.has(harness)) {
        invalid(
          strict,
          'unsupported-context-window',
          `agents.managerContextWindows.${key} is provider-managed and cannot be configured`,
        );
        continue;
      }
      if (raw !== null && (!Number.isSafeInteger(raw) || (raw as number) <= 0)) {
        invalid(
          strict,
          'invalid-context-window',
          `agents.managerContextWindows.${key} must be null or a positive integer`,
        );
        continue;
      }
      contexts[harness] = raw as number | null;
    }
  }

  // Claude owns the legacy marker syntax. Normalize its model and context as
  // one pair so a conflict is refused and an old `opus[1m]` config migrates to
  // `{ managerModels: {claude:'opus'}, managerContextWindows:{claude:1000000} }`.
  const claudeModel = models.claude;
  if (claudeModel?.trim()) {
    const hasContext = Object.prototype.hasOwnProperty.call(contexts, 'claude');
    try {
      const selection = normalizeModelSelection(
        claudeModel,
        hasContext ? contexts.claude : undefined,
      );
      if (
        selection.contextWindow !== null &&
        selection.contextWindow !== 200_000 &&
        selection.contextWindow !== 1_000_000
      ) {
        throw new ManagerSelectionError(
          'unsupported-context-window',
          'Claude manager context must be a validated 200K or 1M model variant',
        );
      }
      if (selection.contextWindow === 200_000 && isClaudeInherentOneMillionModel(selection.model)) {
        throw new ManagerSelectionError(
          'unsupported-context-window',
          `${selection.model} exposes only its inherent 1M context`,
        );
      }
      models.claude = selection.model;
      if (hasContext || selection.contextWindow !== null) {
        contexts.claude = selection.contextWindow;
      }
    } catch (error) {
      if (strict) throw error;
      // Preserve the model when possible and discard only the conflicting or
      // malformed context half. A legacy marker remains recoverable by itself.
      try {
        const selection = normalizeModelSelection(claudeModel);
        models.claude = selection.model;
        if (selection.contextWindow !== null) contexts.claude = selection.contextWindow;
        else delete contexts.claude;
      } catch {
        delete models.claude;
        delete contexts.claude;
      }
    }
  } else if (Object.prototype.hasOwnProperty.call(contexts, 'claude') && contexts.claude !== null) {
    invalid(
      strict,
      'invalid-context-window',
      'agents.managerContextWindows.claude requires a selected Claude model',
    );
    delete contexts.claude;
  }

  return {
    ...(rawModels !== undefined && { managerModels: models }),
    ...(rawEfforts !== undefined && { managerEfforts: efforts }),
    ...(rawContexts !== undefined || Object.keys(contexts).length > 0
      ? { managerContextWindows: contexts }
      : {}),
  };
}

/** True when the map explicitly contains this provider, including `null`. */
export function hasManagerContextPreference(
  map: ManagerContextWindows | undefined,
  harness: Harness,
): boolean {
  return !!map && Object.prototype.hasOwnProperty.call(map, harness);
}

/**
 * Resolve only the manager-specific layer. The generic provider policy is
 * deliberately applied afterwards by providerContext.contextRequestForSpawn.
 */
export function managerContextPreference(
  harness: Harness,
  map: ManagerContextWindows | undefined,
): number | null | undefined {
  if (!CONTEXT_PROVIDERS.has(harness) || !hasManagerContextPreference(map, harness)) {
    return undefined;
  }
  return map![harness];
}

export function managerSelectionErrorCode(error: unknown): string {
  if (error instanceof ManagerSelectionError || error instanceof ModelSelectionError) {
    return error.code;
  }
  return '';
}
