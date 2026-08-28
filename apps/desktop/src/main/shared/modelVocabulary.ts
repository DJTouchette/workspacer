/**
 * Which harness a model id BELONGS to.
 *
 * A model id is never portable between harnesses: `sonnet` means nothing to
 * codex, `gpt-5-codex` means nothing to claude, and an OpenCode id is a
 * `provider/model` pair neither of them parses. Every config field that holds a
 * model — `claude.defaultModel`, `supervisor.model`, `supervisor.summarizerModel`,
 * `agents.autoTitle.model` — predates multi-provider support and therefore holds
 * a CLAUDE id by default. Handing one of those to a codex spawn is a 400 at best
 * and a silently-wrong model at worst.
 *
 * This is the one place that knows the per-harness vocabularies, so a fifth
 * harness is a row here rather than another sweep of the spawn paths. It lives
 * in main/shared because both the spawn resolvers (lib/spawnModel,
 * lib/roleModels) and the one-shot completion adapters (services/directCompletion)
 * need the same answer, and two copies of it would drift.
 *
 * THE RULE THIS ENCODES. `isForeignModel` is deliberately NOT "this harness's
 * whitelist doesn't have it":
 *
 *  - A model only counts as foreign when some OTHER harness positively claims it
 *    and this one does not. `sonnet` under codex is foreign (claude claims it);
 *    `my-finetune-v3` under codex is not (nobody claims it), so it passes
 *    through untouched.
 *  - That matters because these vocabularies are patterns over live, moving CLI
 *    catalogs, not closed sets. A whitelist test would reject every model
 *    shipped after this file was written and silently downgrade a user's
 *    deliberate choice; the foreign test only ever fires on an id we can name
 *    the real owner of.
 *
 * A foreign id resolves to `undefined` — the harness's OWN default — which is
 * the only value valid on every harness.
 */

/**
 * The harnesses, spelled out rather than imported from `services/agentProviders`
 * (which pulls in `fs`/`child_process`): the renderer imports this module too,
 * to grey out a model its picker knows the selected harness cannot serve.
 * Structurally identical to `AgentProvider` — widening one means widening both.
 */
export type Harness = 'claude' | 'codex' | 'opencode' | 'pi';

/**
 * Claude's curated aliases. Concrete ids (`claude-opus-4-5-20251101`) are matched
 * by the `claude-` prefix instead, so this list only has to hold the shorthands
 * the CLI accepts that don't look like model ids.
 */
const CLAUDE_ALIASES = new Set([
  'default',
  'haiku',
  'sonnet',
  'sonnet[1m]',
  'opus',
  'opusplan',
  'fable',
]);

/**
 * True when `model` is in `provider`'s vocabulary.
 *
 * Patterns, not catalogs: asking a harness for its real model list means booting
 * its CLI, which no spawn path can afford synchronously. These are verified
 * against each installed CLI's accepted `--model` values.
 */
const VOCABULARY: Record<Harness, (model: string) => boolean> = {
  // Aliases (with the `[1m]` context-window suffix claude alone uses) plus any
  // concrete `claude-*` id.
  claude: (m) => CLAUDE_ALIASES.has(m.toLowerCase()) || /^claude-/i.test(m),
  // `gpt-5.1-codex-max`, `o3`, `codex-mini`, `gpt4` — codex's own families.
  codex: (m) => /^(gpt-|o\d|codex-|gpt\d)/i.test(m),
  // OpenCode's `--model` is always `provider/model` (`anthropic/claude-sonnet-4`).
  opencode: (m) => /^[\w.-]+\/[\w.:-]+$/.test(m),
  // Pi uses the same `provider/model` form.
  pi: (m) => /^[\w.-]+\/[\w.:-]+$/.test(m),
};

const PROVIDERS = Object.keys(VOCABULARY) as Harness[];

/** True when `provider` can serve `model`. Blank/unknown provider ⇒ false. */
export function servesModel(provider: string, model: string | null | undefined): boolean {
  const id = (model ?? '').trim();
  if (!id) return false;
  return VOCABULARY[provider as Harness]?.(id) ?? false;
}

/**
 * Every harness that claims `model`. Empty when nobody does — an id we have no
 * opinion about, which is NOT the same as an invalid one.
 *
 * Note `opencode` and `pi` share the `provider/model` form, so a slash id is
 * claimed by both. That is honest: we genuinely cannot tell them apart from the
 * string, and the foreign test below stays correct either way.
 */
export function providersServing(model: string | null | undefined): Harness[] {
  const id = (model ?? '').trim();
  if (!id) return [];
  return PROVIDERS.filter((p) => VOCABULARY[p](id));
}

/**
 * True when `model` demonstrably belongs to a DIFFERENT harness than `provider`.
 *
 * The guard every spawn path applies before putting a configured model on a
 * CLI's argv. False for an id nobody claims (pass it through — see the header)
 * and false for a blank one (there is nothing to be wrong about).
 */
export function isForeignModel(provider: string, model: string | null | undefined): boolean {
  const id = (model ?? '').trim();
  if (!id) return false;
  if (servesModel(provider, id)) return false;
  return providersServing(id).length > 0;
}
