/**
 * Which model an INTERNAL ROLE spawn asks for, per harness.
 *
 * Three roles beside the supervisor (lib/supervisorModel) choose a model on the
 * user's behalf rather than taking one from a caller: the Fleet Manager, the
 * supervisor's transcript-digest workers, and the auto-title one-shot. All three
 * had the same shape of bug, in three stages of severity:
 *
 *  - The Fleet Manager had NO model setting at all (`agents.managerProvider`
 *    existed with no twin), so it always ran on its harness's default with no
 *    way to choose.
 *  - `supervisor.summarizerModel` shipped `'sonnet'` — a claude id — and the
 *    /supervise prompt handed it to `spawn_agent` verbatim. On a codex
 *    supervisor that is an id the codex CLI rejects.
 *  - `agents.autoTitle.model` shipped `'haiku'`, also a claude id, but the
 *    titler already downgraded an unservable one; what it lacked was a way for
 *    a codex-primary user to NAME a codex title model.
 *
 * So each is a per-harness map keyed by provider, the same shape
 * `supervisor.models` settled on in the commit that made the supervisor picker
 * provider-aware, and resolution is uniformly:
 *
 *  1. `<map>[provider]` — what the user chose on THIS harness.
 *  2. the legacy single field, but ONLY when this harness can actually serve it
 *     (`modelVocabulary.isForeignModel`) — that is what stops `'sonnet'` from
 *     reaching codex while still honouring it for claude.
 *  3. undefined — the harness picks its own default, the one value valid
 *     everywhere.
 *
 * Never guesses across harnesses. A blank here is always safe; a foreign id
 * never is.
 */
import { configService } from '../services/configService';
import type { AgentProvider } from '../services/agentProviders';
import { isForeignModel } from '../shared/modelVocabulary';

/**
 * The shared resolution above, as one function. Exported because it is the
 * seam worth pinning: every role reduces to this plus which fields it reads.
 *
 * `legacy` is the pre-multi-provider single field. It is accepted only when it
 * is not demonstrably another harness's id — an id nobody claims still passes,
 * because these vocabularies are patterns over live catalogs and refusing an
 * unrecognized id would silently discard a deliberate choice.
 */
export function perHarnessModel(
  provider: AgentProvider,
  map: Record<string, string> | undefined,
  legacy?: string,
): string | undefined {
  const chosen = map?.[provider];
  if (typeof chosen === 'string' && chosen.trim()) return chosen.trim();
  const fallback = (legacy ?? '').trim();
  if (!fallback || isForeignModel(provider, fallback)) return undefined;
  return fallback;
}

/**
 * Model for the FLEET MANAGER's own conversation (`agents.managerModels`).
 *
 * Resolved in main rather than passed from the renderer so it lands on EVERY
 * way a manager starts — the Overview hero, the command palette, a respawn of a
 * stopped manager card, a headless bus spawn — instead of only the one entry
 * point that remembered to read the config. There is no legacy single field:
 * the setting is new, so it was born per-harness.
 */
export function resolveManagerModel(provider: AgentProvider): string | undefined {
  const agents = configService.getConfig().agents as
    { managerModels?: Record<string, string> } | undefined;
  return perHarnessModel(provider, agents?.managerModels);
}

/**
 * Model for the supervisor's transcript-DIGEST workers
 * (`supervisor.summarizerModels`, legacy `supervisor.summarizerModel`).
 *
 * `provider` is the harness the digest worker will RUN on, which is now the
 * supervisor's own (see mcpConfig's `summarizerProvider`) — previously those
 * workers were spawned through the facade with no provider named at all, so a
 * codex supervisor dispatched Claude summarizers and the claude-only default
 * looked correct by accident.
 */
export function resolveSummarizerModel(provider: AgentProvider): string | undefined {
  const sup = configService.getConfig().supervisor as
    { summarizerModel?: string; summarizerModels?: Record<string, string> } | undefined;
  return perHarnessModel(provider, sup?.summarizerModels, sup?.summarizerModel);
}

/**
 * Model for the AUTO-TITLE one-shot (`agents.autoTitle.models`, legacy
 * `agents.autoTitle.model`), for the harness the titled agent itself runs on.
 *
 * Unlike the other two this is not a single configured harness — every agent is
 * titled by its OWN provider — so the map is the whole point rather than a
 * memory of a picker: a mixed fleet needs a claude title model AND a codex one
 * live at the same time.
 */
export function resolveTitleModel(provider: AgentProvider): string | undefined {
  const auto = configService.getConfig().agents?.autoTitle as
    { model?: string; models?: Record<string, string> } | undefined;
  return perHarnessModel(provider, auto?.models, auto?.model);
}
