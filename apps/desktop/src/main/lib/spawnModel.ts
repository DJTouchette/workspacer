/**
 * What model a spawn is actually asking for, when the caller named none.
 *
 * THE DROP THIS CLOSES. `config.claude.defaultModel` ships as `opus[1m]`, and
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
 */
import { configService } from '../services/configService';

export function resolveSpawnModel(
  provider: string,
  requested: string | null | undefined,
): string | undefined {
  const explicit = requested?.trim();
  if (explicit) return explicit;
  if (provider !== 'claude') return undefined;
  const configured = configService.getConfig().claude?.defaultModel;
  return typeof configured === 'string' && configured.trim() ? configured.trim() : undefined;
}
