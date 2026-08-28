/**
 * Which model a SUPERVISOR spawn asks for when the caller named none.
 *
 * `supervisor.model` is a single field, but the supervisor can run on any of
 * three harnesses (Settings → Supervisor → Supervisor agent), and a model id is
 * never portable between them: `fable` means nothing to codex, `gpt-5-codex`
 * means nothing to Claude. The old reader was `supCfg?.model` inline in the PTY
 * Claude path, which had two bugs waiting in it — a codex supervisor ignored the
 * setting entirely (it never reached managedSpawn), and a Claude supervisor
 * launched from "Ask the Fleet" while `supervisor.provider` was codex would have
 * picked up the codex id and 400'd at spawn.
 *
 * So the config keeps a per-provider memory (`supervisor.models`, written by the
 * settings picker) beside the active `supervisor.model`, and resolution is:
 *
 *  1. `supervisor.models[provider]` — what the user last chose on THIS harness.
 *  2. `supervisor.model`, but only when `supervisor.provider` is this harness —
 *     the legacy single field, which is only meaningful for its own provider.
 *  3. undefined — the harness picks its own default.
 *
 * Never guesses across providers: an unset value here means "the CLI's default",
 * which is always valid, where a foreign id never is.
 */
import { configService } from '../services/configService';
import type { AgentProvider } from '../services/agentProviders';

export function resolveSupervisorModel(provider: AgentProvider): string | undefined {
  const sup = configService.getConfig().supervisor as
    { model?: string; models?: Record<string, string>; provider?: string } | undefined;
  const perProvider = sup?.models?.[provider];
  if (typeof perProvider === 'string' && perProvider.trim()) return perProvider.trim();
  const configuredProvider = (sup?.provider || 'claude').trim() || 'claude';
  if (configuredProvider !== provider) return undefined;
  const model = sup?.model;
  return typeof model === 'string' && model.trim() ? model.trim() : undefined;
}
