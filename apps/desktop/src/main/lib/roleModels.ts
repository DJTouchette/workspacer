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
import { managerContextPreference, type ManagerContextWindows } from '../shared/managerSelection';

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

/**
 * Reasoning EFFORT for an internal role, per harness
 * (`agents.managerEfforts`).
 *
 * Same per-harness shape as the model maps and for the same reason: the effort
 * ladders are not portable either (claude's is low|medium|high|max, codex's is
 * minimal…xhigh, copilot's is its own seven), so one shared field would push a
 * level at a CLI that has never heard of it. There is no legacy single field —
 * these settings were born per-harness — so an unknown value is simply passed
 * through: the ladders come from live catalogs (providerCaps), and refusing an
 * unrecognized id would discard a deliberate choice on a newer CLI.
 *
 * Blank = the harness's own default, which is what the manager ran on before:
 * its effort was un-settable, so a codex manager coordinated the whole fleet at
 * whatever `codex` defaults to with no way to raise it.
 */
function perHarnessEffort(
  provider: AgentProvider,
  map: Record<string, string> | undefined,
): string | undefined {
  const chosen = map?.[provider];
  return typeof chosen === 'string' && chosen.trim() ? chosen.trim() : undefined;
}

/** Effort for a FLEET MANAGER spawn on `provider` (`agents.managerEfforts`). */
export function resolveManagerEffort(provider: AgentProvider): string | undefined {
  const agents = configService.getConfig().agents as
    { managerEfforts?: Record<string, string> } | undefined;
  return perHarnessEffort(provider, agents?.managerEfforts);
}

/** Requested window for a NEW Fleet Manager life on this harness. The caller
 *  applies the generic provider policy afterwards, so an absent Codex entry
 *  becomes the shared fresh 1M request while an explicit null stays
 *  provider-default. Resume callers deliberately do not consult this setting. */
export function resolveManagerContextWindow(provider: AgentProvider): number | null | undefined {
  const agents = configService.getConfig().agents as
    { managerContextWindows?: ManagerContextWindows } | undefined;
  return managerContextPreference(provider, agents?.managerContextWindows);
}

/** explicit call > stored manager preference > provider policy. A resume with
 *  no durable explicit value skips the preference so an old conversation is
 *  never silently rewritten by a later Settings change. */
export function resolveManagerContextForSpawn(
  provider: AgentProvider,
  explicit: number | null | undefined,
  resumeSessionId?: string,
): number | null | undefined {
  if (explicit !== undefined) return explicit;
  if (resumeSessionId) return undefined;
  return resolveManagerContextWindow(provider);
}
