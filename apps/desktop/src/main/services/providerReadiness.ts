import type { AgentProvider } from './agentProviders';
import type { CompletionResult } from './directCompletion';
import {
  type ProviderReadiness,
  UNCHECKED_PROVIDER,
  UNSUPPORTED_PROVIDER,
} from '../shared/providerReadiness';

export interface ReadinessContext {
  /** Private local identity; never returned over IPC. */
  key: string;
  provider: string;
  bin: string | null;
  local: boolean;
  enabled: boolean;
}
export function completionReadiness(result: CompletionResult, now: number): ProviderReadiness {
  if (result.ok) return { state: 'responding', checkedAt: now };
  const state =
    result.reason === 'not-authed'
      ? 'unauthenticated'
      : result.reason === 'rate-limited'
        ? 'limited'
        : result.reason === 'timeout'
          ? 'timeout'
          : result.reason === 'cancelled' || result.reason === 'binary-missing'
            ? 'unchecked'
            : [
                  'unsupported-provider',
                  'unsupported-model',
                  'no-tools-unsupported',
                  'daemon-unavailable',
                ].includes(result.reason)
              ? 'unsupported'
              : /ECONN|ENOTFOUND|EAI_AGAIN|network|connection|fetch failed/i.test(result.message)
                ? 'network-error'
                : 'error';
  return { state, checkedAt: now };
}

/** One scheduler per desktop process. Reads/remounts never spend allowance.
 * Startup runs once for the selected manager, with no retries or provider fallback.
 * Config/owner changes cancel work and make all prior facts inaccessible.
 */
export class ProviderReadinessService {
  private started = false;
  private epoch = 0;
  private timer?: ReturnType<typeof setTimeout>;
  private entries = new Map<
    string,
    {
      result: ProviderReadiness;
      at: number;
      flight?: Promise<ProviderReadiness>;
      controller?: AbortController;
    }
  >();
  constructor(
    private deps: {
      context: (provider?: string) => ReadinessContext;
      ping: (
        provider: AgentProvider,
        bin: string,
        signal: AbortSignal,
      ) => Promise<CompletionResult>;
      now?: () => number;
    },
  ) {}
  start(): void {
    if (this.started) return;
    this.started = true;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      const ctx = this.deps.context();
      if (ctx.enabled && ctx.local) void this.check(ctx.provider, true);
    }, 2000);
  }
  invalidate(): void {
    this.epoch++;
    for (const entry of this.entries.values()) entry.controller?.abort();
    this.entries.clear();
    // Delayed startup always reads current config again, including opt-out.
  }
  dispose(): void {
    clearTimeout(this.timer);
    this.invalidate();
  }
  read(provider: string): ProviderReadiness {
    const ctx = this.deps.context(provider);
    if (!ctx.local) return UNSUPPORTED_PROVIDER;
    return this.entries.get(ctx.key)?.result ?? UNCHECKED_PROVIDER;
  }
  async check(provider: string, automatic = false): Promise<ProviderReadiness> {
    const ctx = this.deps.context(provider);
    if (!ctx.local) return UNSUPPORTED_PROVIDER;
    if (automatic && !ctx.enabled) return UNCHECKED_PROVIDER;
    if (!ctx.bin) return UNCHECKED_PROVIDER;
    const now = this.deps.now ?? Date.now;
    const existing = this.entries.get(ctx.key);
    if (existing?.flight) return existing.flight;
    if (automatic && existing) return existing.result;
    // Coalesce near-simultaneous refresh buttons in different windows.
    if (existing && now() - existing.at < 2000) return existing.result;
    const epoch = this.epoch;
    const controller = new AbortController();
    const entry: {
      result: ProviderReadiness;
      at: number;
      flight?: Promise<ProviderReadiness>;
      controller?: AbortController;
    } = {
      result: { state: 'checking' },
      at: now(),
      controller,
    };
    this.entries.set(ctx.key, entry);
    entry.flight = (async () => {
      let deadline: ReturnType<typeof setTimeout> | undefined;
      let result: ProviderReadiness;
      try {
        result = await Promise.race([
          this.deps
            .ping(provider as AgentProvider, ctx.bin!, controller.signal)
            .then((r) => completionReadiness(r, now())),
          new Promise<ProviderReadiness>((resolve) => {
            deadline = setTimeout(() => {
              controller.abort();
              resolve({ state: 'timeout', checkedAt: now() });
            }, 20000);
          }),
        ]);
      } catch {
        result = { state: 'error', checkedAt: now() };
      } finally {
        clearTimeout(deadline);
      }
      if (epoch !== this.epoch || ctx.key !== this.deps.context(provider).key)
        return UNCHECKED_PROVIDER;
      entry.result = result;
      entry.at = now();
      entry.flight = undefined;
      entry.controller = undefined;
      return result;
    })();
    return entry.flight;
  }
}
