/**
 * Which HARNESS an internal-role spawn runs on when the caller named none.
 *
 * `agents.managerProvider` is the setting that says "the manager runs on
 * claude". It was read in exactly ONE place — a renderer component at the entry
 * point that happened to remember it (App's fleet-manager:ask listener) — and
 * nowhere in main. Every other way the role starts therefore fell through to
 * `provider ?? 'claude'` at the two spawn funnels (`claude:spawn` in ipc.ts,
 * `agents.spawn` in hubCapabilities.ts):
 *
 *   - a manager spawned over the hub bus (the web /app client, the /m PWA, a
 *     hub job, a federated peer) — the bus payload carries `manager: true`
 *     with no provider,
 *   - a respawn whose stored card predates the provider field,
 *   - the next entry point somebody adds.
 *
 * All of them silently produced a CLAUDE manager while Settings said codex,
 * and a silently-wrong harness looks exactly like a working one. This is the
 * same class of bug — and the same fix — as lib/roleModels: resolve the role's
 * config in MAIN, once, so it lands on every path instead of only the one that
 * remembered to read it.
 *
 * Rules:
 *  1. An explicit provider from the caller always wins (picking codex in the
 *     "Ask the Fleet" launcher must override a claude default in Settings).
 *  2. Otherwise the role's configured harness.
 *  3. Otherwise 'claude', the shipped default.
 *
 * A value config does not recognize is ignored rather than passed on: an
 * unknown provider string reaches an adapter that has no idea what it is, where
 * 'claude' at least runs.
 */
import { configService } from '../services/configService';
import type { AgentProvider } from '../services/agentProviders';

/** Every harness id, as a runtime lookup. Typed as a total Record so adding a
 *  provider to `AgentProvider` fails to compile until it is listed here — the
 *  cheap version of a validator that silently stops recognizing a new harness.
 *  (A `Record` rather than an import of a value from services/agentProviders:
 *  this module is reached from the spawn funnels, whose tests stub that service,
 *  and a type-only import survives the stub.) */
const KNOWN_PROVIDERS: Record<AgentProvider, true> = {
  claude: true,
  codex: true,
  copilot: true,
  opencode: true,
  pi: true,
};

/** Normalize an untrusted provider string; anything else reads as "unstated". */
function parseProvider(value: unknown): AgentProvider | undefined {
  return typeof value === 'string' && value in KNOWN_PROVIDERS
    ? (value as AgentProvider)
    : undefined;
}

/** The harness the FLEET MANAGER runs on (`agents.managerProvider`). */
export function resolveManagerProvider(): AgentProvider {
  const agents = configService.getConfig().agents as { managerProvider?: string } | undefined;
  return parseProvider(agents?.managerProvider?.trim()) ?? 'claude';
}

/**
 * The provider a spawn request resolves to, given its role flags. Called by
 * both spawn funnels so the IPC and hub-bus paths cannot disagree about which
 * CLI the manager runs on — the standing rule for anything spawn-shaped here.
 */
export function resolveSpawnProvider(req: { provider?: string; manager?: boolean }): AgentProvider {
  const explicit = parseProvider(req.provider?.trim());
  if (explicit) return explicit;
  if (req.manager) return resolveManagerProvider();
  return 'claude';
}
