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
import { normalizeModelSelection, type ModelSelection } from '../shared/modelContextWindows';

export function resolveSpawnModelSelection(
  provider: string,
  requested: string | null | undefined,
): ModelSelection | undefined {
  const explicit = requested?.trim();
  if (explicit) {
    if (!isForeignModel(provider, explicit)) {
      // `[1m]` / `-1m` are Claude's legacy window syntax, not a cross-provider
      // model-id convention. Other harnesses own their ids verbatim.
      return provider === 'claude'
        ? normalizeModelSelection(explicit)
        : { model: explicit, contextWindow: null };
    }
    console.log(
      `[spawnModel] dropping model '${explicit}' from a ${provider} spawn — it belongs to ` +
        `another harness; using ${provider}'s own default instead`,
    );
    return undefined;
  }
  if (provider !== 'claude') return undefined;
  const configured = configService.getConfig().claude;
  if (typeof configured?.defaultModel !== 'string' || !configured.defaultModel.trim()) {
    return undefined;
  }
  return normalizeModelSelection(configured.defaultModel, configured.contextWindow);
}

export function resolveSpawnModel(
  provider: string,
  requested: string | null | undefined,
): string | undefined {
  return resolveSpawnModelSelection(provider, requested)?.model;
}
