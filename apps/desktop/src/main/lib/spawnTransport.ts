/**
 * Which TRANSPORT a spawn runs on when the caller named none.
 *
 * Two harnesses have two session shapes each, and the shapes are twins:
 *
 *   claude  'pty'    the classic Claude Code TUI in a PTY (Term + GUI)
 *           'stream' headless `--print --output-format stream-json` (GUI only)
 *   codex   'pty'    the hybrid: the native Codex TUI in a PTY + the structured
 *                    GUI, both on one shared `codex app-server` thread
 *           'stream' headless `codex app-server` over ws (GUI only)
 *
 * Before this module the default lived as `opts.transport ?? cfg.claude.transport
 * ?? 'pty'` copy-pasted at four separate spawn entry points, and codex had no
 * configured default at all — an absent transport just meant "hybrid", spelled
 * as the ABSENCE of a key. That is why a codex spawn came up GUI-only on one
 * machine and as a TUI+viewer pair on another: nothing in the request said what
 * was wanted, so every layer guessed, and the guesses differed by platform.
 *
 * So the rule is stated once, here, and read by every path:
 *
 *   1. what the caller explicitly asked for ('pty' | 'stream'), else
 *   2. `config.<provider>.transport` (claude.transport / codex.transport), else
 *   3. the shipped fallback below.
 *
 * Providers with only one session shape (opencode, pi) get `undefined` — they
 * take no transport at all, and inventing one for them would put a key on a
 * payload their adapter does not read.
 *
 * TWIN: the headless Go brain resolves the same thing in
 * `services/hub/cmd/brain/handlers.go` (`transportDefault`), because a spawn
 * that never touches the desktop must land on the same shape.
 */
import { configService } from '../services/configService';
import type { AgentProvider } from '../services/agentProviders';

export type AgentTransport = 'pty' | 'stream';

/**
 * What each harness runs on when neither the caller nor config says. Mirrors
 * `config_defaults.json` — a fresh install reads its default from config, so
 * these only matter for a config that predates the key (an existing user's
 * `config.yaml` has no `codex` section at all) or a test with an empty config.
 */
export const TRANSPORT_FALLBACK: Readonly<Record<'claude' | 'codex', AgentTransport>> = {
  // Claude's headless transport is opt-in per install (config_defaults ships
  // 'stream', but a long-lived config.yaml may still say 'pty').
  claude: 'pty',
  // Codex is headless by default: the app-server path is the one that mirrors
  // Claude's stream transport — GUI-only, daemon-owned thread, no PTY.
  codex: 'stream',
};

/** Whether this provider HAS a transport choice at all. */
export function hasTransportChoice(provider: string): provider is 'claude' | 'codex' {
  return provider === 'claude' || provider === 'codex';
}

/** Normalize an untrusted transport string; anything else reads as "unstated". */
export function parseTransport(value: unknown): AgentTransport | undefined {
  return value === 'pty' || value === 'stream' ? value : undefined;
}

/** The two config sections this reads, as much of them as it needs. */
interface TransportConfig {
  claude?: { transport?: unknown };
  codex?: { transport?: unknown };
}

/**
 * The transport a spawn should run on. `requested` wins; otherwise the
 * harness's configured default; otherwise the shipped fallback. Returns
 * undefined for a provider that has no transport choice, so callers can keep
 * the key OFF payloads that must not carry it.
 *
 * `cfg` lets a caller that already holds a config snapshot pass it in rather
 * than provoke a second read: a spawn handler resolves several config-derived
 * defaults in a row (bypass, model, transport), and re-reading between them is
 * how one spawn ends up half-decided by the config before a concurrent write
 * and half by the config after it.
 */
export function resolveTransport(
  provider: AgentProvider | string,
  requested?: string,
  cfg?: TransportConfig,
): AgentTransport | undefined {
  if (!hasTransportChoice(provider)) return undefined;
  const explicit = parseTransport(requested);
  if (explicit) return explicit;
  const config = cfg ?? configService.getConfig();
  // Named per harness rather than indexed: the two sections are typed, and a
  // dynamic lookup would happily read `config.opencode.transport` (which does
  // not exist) if this were ever called with a widened provider string.
  const configured = provider === 'claude' ? config.claude?.transport : config.codex?.transport;
  return parseTransport(configured) ?? TRANSPORT_FALLBACK[provider];
}
