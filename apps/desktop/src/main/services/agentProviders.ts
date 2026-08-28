/**
 * Provider registry (launch side) for multi-agent support — see
 * docs/multi-agent-providers.md.
 *
 * Claude delegates to the existing claudeResolver (full flag set). Codex and
 * OpenCode are, for now, launched as their own interactive TUIs inside the PTY
 * (Tier-1): we resolve the binary on PATH and run it in the agent's cwd. The
 * richer "managed" integration (driving `codex app-server` / `opencode serve`
 * and translating their events into the session model) lands in later phases.
 */
import * as path from 'path';
import * as fs from 'fs';
import { buildClaudeArgv, ClaudeArgvOptions } from './claudeResolver';

export type AgentProvider = 'claude' | 'codex' | 'copilot' | 'opencode' | 'pi';

/** Detection result for one provider. */
export interface ProviderStatus {
  provider: AgentProvider;
  found: boolean;
  /** Absolute path if detected; null if the binary is missing. */
  resolvedPath: string | null;
  /** The user-configured custom binary path (may be empty string = not set). */
  customBin: string;
}

/** True when `p` names something that could actually be exec'd: it exists and is
 *  not a DIRECTORY. `existsSync` alone accepted a directory, and the Go twin
 *  (cmd/brain/providers.go findOnPath: `os.Stat(full); err == nil && !st.IsDir()`)
 *  did not — so with a directory named `codex` earlier on PATH than the real
 *  binary, the brain skipped it and kept scanning while this side returned the
 *  DIRECTORY as argv[0] of every codex spawn, and the Spawn dialog showed a green
 *  detection dot for a provider that cannot launch. */
function isExecutableCandidate(p: string): boolean {
  try {
    return !fs.statSync(p).isDirectory();
  } catch {
    return false; // missing, dangling symlink, unreadable — all "not a binary"
  }
}

/** First existing absolute path for any of `names` across PATH, else null. */
function findOnPath(names: string[]): string | null {
  const dirs = (process.env.PATH || '').split(path.delimiter).filter(Boolean);
  for (const dir of dirs) {
    for (const name of names) {
      const full = path.join(dir, name);
      if (isExecutableCandidate(full)) return full;
    }
  }
  return null;
}

/** Candidate binary names per provider, platform-aware. */
function binNames(base: string): string[] {
  return process.platform === 'win32' ? [`${base}.cmd`, `${base}.exe`, base] : [base];
}

/**
 * Resolve the launcher binary for a provider. When `customBin` is supplied and
 * non-empty it is returned directly (the user's configured override). Otherwise
 * falls back to a PATH search, then to the bare command name so a freshly-
 * installed CLI still works without a restart.
 */
export function resolveAgentBinary(provider: AgentProvider, customBin?: string): string {
  if (customBin?.trim()) return customBin.trim();
  // Binary name matches the provider id for every CLI (claude/codex/copilot/opencode/pi).
  return findOnPath(binNames(provider)) ?? provider;
}

/**
 * True if the provider's CLI is accessible right now. When `customBin` is set
 * we check that path directly; otherwise we search PATH.  PATH is read fresh,
 * so a just-installed CLI is detected without a restart.
 */
export function isAgentBinaryInstalled(provider: AgentProvider, customBin?: string): boolean {
  if (customBin?.trim()) return isExecutableCandidate(customBin.trim());
  return findOnPath(binNames(provider)) !== null;
}

/** Check detection status for all providers (including Claude). `binaries` maps
 *  provider id → user-configured override path ('' = not set). */
export function checkAllProviders(
  binaries: Partial<Record<AgentProvider, string>> = {},
): ProviderStatus[] {
  const all: AgentProvider[] = ['claude', 'codex', 'copilot', 'opencode', 'pi'];
  return all.map((provider) => {
    const customBin = (binaries[provider] ?? '').trim();
    if (customBin) {
      const found = isExecutableCandidate(customBin);
      return { provider, found, resolvedPath: found ? customBin : null, customBin };
    }
    const resolvedPath = findOnPath(binNames(provider));
    return { provider, found: resolvedPath !== null, resolvedPath, customBin: '' };
  });
}

/**
 * Rescan window for {@link checkAllProvidersCached}. Every provider picker in
 * the app now asks on open (renderer hook useProviderDetection) so it can hide
 * harnesses that aren't installed — that turns one PATH walk per provider into
 * a call the UI makes routinely, and PATH can be long. Binaries don't move
 * often, so a few seconds of staleness buys a free reopen while still letting a
 * CLI installed mid-session appear without a restart.
 */
const DETECTION_TTL_MS = 5000;

let detectionCache: { key: string; at: number; value: ProviderStatus[] } | null = null;

/**
 * {@link checkAllProviders} with a short TTL. The cache key includes the
 * binary overrides, so editing `agents.binaries` is answered by a fresh scan
 * rather than by the previous override's result.
 */
export function checkAllProvidersCached(
  binaries: Partial<Record<AgentProvider, string>> = {},
  force = false,
): ProviderStatus[] {
  const key = JSON.stringify(
    Object.entries(binaries)
      .map(([k, v]) => [k, (v ?? '').trim()])
      .sort(([a], [b]) => a.localeCompare(b)),
  );
  const now = Date.now();
  if (
    !force &&
    detectionCache &&
    detectionCache.key === key &&
    now - detectionCache.at < DETECTION_TTL_MS
  ) {
    return detectionCache.value;
  }
  const value = checkAllProviders(binaries);
  detectionCache = { key, at: now, value };
  return value;
}

export interface AgentArgvOptions extends ClaudeArgvOptions {
  /** The agent backend to launch. Defaults to 'claude'. */
  provider?: AgentProvider;
}

/**
 * Build the argv for a provider. Claude gets the full resolver treatment;
 * Codex/OpenCode get a minimal interactive-TUI launch (Tier-1) — model/session
 * flags are intentionally omitted here because those are Claude-CLI specific and
 * the two other CLIs manage model/session through their own config until the
 * managed adapters land.
 */
export function buildAgentArgv(opts: AgentArgvOptions = {}): string[] {
  const provider = opts.provider ?? 'claude';
  if (provider === 'claude') return buildClaudeArgv(opts);
  return [resolveAgentBinary(provider)];
}
