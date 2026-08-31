/**
 * What model a spawn is actually asking for, when the caller named none.
 *
 * THE DROP THIS CLOSES. `config.claude.defaultModel` ships as `opus` with
 * `claude.contextWindow: 1000000`, and
 * `SpawnAgentDialog` prefills the picker from it — so a human clicking Spawn
 * sends an explicit model and the daemon records it. Every OTHER entry point
 * leaves it undefined: `App.tsx`'s restore path, `agents.spawn` over the hub bus
 * (the MCP facade, /m, a Fleet Manager dispatching a worker), jobs. Claude Code
 * then resolves its own default internally and the daemon never learns which
 * one, so `requested_model` is empty and the `[1m]` marker — the ONLY carrier of
 * a 1M choice before the provider reports a window, because Claude Code strips
 * it from the id it writes into the transcript — is lost.
 *
 * That is the dispatched-worker case, which is most of the fleet.
 *
 * Resolving it here does two things at once: the daemon gets the model string to
 * record, and the CLI gets it on its argv, so what we recorded is what actually
 * ran rather than a hopeful guess about someone else's default.
 *
 * Only `claude` has a configured default (`config.claude.defaultModel`); the
 * other providers carry their own and are returned unchanged. An unset default
 * still resolves to undefined — this fills a blank, it never overrides a caller.
 *
 * THE SECOND DROP THIS CLOSES: a model that belongs to a DIFFERENT harness.
 * Every spawn path funnels through here, so this is the last place a `sonnet`
 * can be stopped before it reaches a codex argv. The caller's explicit value
 * used to be trusted absolutely — correct for a picker sourced from the
 * provider's own catalog, wrong for the callers that are NOT pickers: the MCP
 * facade's `spawn_agent` (a supervisor relaying a configured summarizer id), a
 * hub job's hand-written spec, a respawn replaying a record written before the
 * agent's provider changed. A foreign id is dropped to undefined rather than
 * forwarded, because the harness's own default is the one value valid
 * everywhere; an id NOBODY claims is still passed through untouched, since
 * these vocabularies are patterns over live catalogs and refusing an
 * unrecognized model would discard a deliberate choice (see modelVocabulary).
 */
import { configService } from '../services/configService';
import { isForeignModel } from '../shared/modelVocabulary';
import {
  claudeArgvModel,
  ModelSelectionError,
  normalizeModelSelection,
  sameModelSelection,
  type ModelSelection,
} from '../shared/modelContextWindows';

export interface SpawnModelInput {
  /** Marker-bearing compatibility spelling understood by old peers/daemons. */
  model?: string | null;
  /** Canonical provider identity. Never marker-decorated by a new writer. */
  modelIdentity?: string | null;
  /** Explicit selected window. Absence remains unknown, never implicit 200K. */
  contextWindow?: number | null;
}

function validateWindow(contextWindow: number | null | undefined): number | null {
  if (contextWindow == null) return null;
  if (!Number.isSafeInteger(contextWindow) || contextWindow <= 0) {
    // Reuse the shared error/code boundary rather than grow a second validator.
    normalizeModelSelection('model', contextWindow);
  }
  return contextWindow;
}

/**
 * Resolve the additive spawn wire. A present canonical pair wins, while a
 * marker-only legacy caller remains valid. When both generations are present
 * they must describe the same selection: otherwise a new receiver and an old
 * receiver would launch different agents from the same request.
 *
 * Claude alone owns `[1m]` syntax. Other providers keep ids such as
 * `vendor/model-1m` byte-for-byte and never acquire a Claude marker.
 */
export function resolveSpawnModelInput(
  provider: string,
  input: SpawnModelInput,
): ModelSelection | undefined {
  const normalizedProvider = provider.toLowerCase();
  const legacy = input.model?.trim() || '';
  const identity = input.modelIdentity?.trim() || '';
  const hasCanonical = identity !== '' || input.contextWindow != null;

  if (normalizedProvider !== 'claude') {
    const contextWindow = validateWindow(input.contextWindow);
    if (hasCanonical && !identity && !legacy) {
      throw new ModelSelectionError(
        'empty-model',
        'modelIdentity or legacy model is required when contextWindow is present',
      );
    }
    if (!identity && !legacy) return undefined;
    if (identity && legacy && identity !== legacy) {
      throw new ModelSelectionError(
        'conflicting-model-identity',
        'modelIdentity conflicts with the legacy model companion',
      );
    }
    const model = identity || legacy;
    if (isForeignModel(normalizedProvider, model)) {
      console.log(
        `[spawnModel] dropping model '${model}' from a ${provider} spawn — it belongs to ` +
          `another harness; using ${provider}'s own default instead`,
      );
      return undefined;
    }
    return { model, contextWindow };
  }

  let selection: ModelSelection | undefined;
  if (hasCanonical) {
    if (!identity && !legacy) {
      throw new ModelSelectionError(
        'empty-model',
        'modelIdentity or legacy model is required when contextWindow is present',
      );
    }
    selection = normalizeModelSelection(identity || legacy, input.contextWindow);
    if (identity && selection.model !== identity) {
      throw new ModelSelectionError(
        'conflicting-model-identity',
        'modelIdentity must be canonical and cannot contain a legacy marker',
      );
    }
    if (identity && legacy) {
      const compatibility = normalizeModelSelection(legacy);
      const expectedCompatibility = normalizeModelSelection(claudeArgvModel(selection));
      if (!sameModelSelection(expectedCompatibility, compatibility)) {
        throw new ModelSelectionError(
          'conflicting-model-identity',
          'canonical model selection conflicts with the legacy model companion',
        );
      }
    }
  } else if (legacy) {
    selection = normalizeModelSelection(legacy);
  }
  if (!selection) return undefined;
  if (isForeignModel(normalizedProvider, selection.model)) {
    console.log(
      `[spawnModel] dropping model '${selection.model}' from a ${provider} spawn — it belongs to ` +
        `another harness; using ${provider}'s own default instead`,
    );
    return undefined;
  }
  return selection;
}

export function resolveSpawnModelSelection(
  provider: string,
  requested: string | null | undefined,
  modelIdentity?: string | null,
  contextWindow?: number | null,
): ModelSelection | undefined {
  const explicit = resolveSpawnModelInput(provider, {
    model: requested,
    modelIdentity,
    contextWindow,
  });
  if (explicit) return explicit;
  if (requested?.trim() || modelIdentity?.trim() || contextWindow != null) return undefined;
  if (provider.toLowerCase() !== 'claude') return undefined;
  const configured = configService.getConfig().claude;
  if (typeof configured?.defaultModel !== 'string' || !configured.defaultModel.trim()) {
    return undefined;
  }
  return normalizeModelSelection(configured.defaultModel, configured.contextWindow);
}
