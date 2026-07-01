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

export type AgentProvider = 'claude' | 'codex' | 'opencode' | 'pi';

/** Detection result for one provider. */
export interface ProviderStatus {
  provider: AgentProvider;
  found: boolean;
  /** Absolute path if detected; null if the binary is missing. */
  resolvedPath: string | null;
  /** The user-configured custom binary path (may be empty string = not set). */
  customBin: string;
}

/** First existing absolute path for any of `names` across PATH, else null. */
function findOnPath(names: string[]): string | null {
  const dirs = (process.env.PATH || '').split(path.delimiter).filter(Boolean);
  for (const dir of dirs) {
    for (const name of names) {
      const full = path.join(dir, name);
      try {
        if (fs.existsSync(full)) return full;
      } catch {
        /* unreadable PATH entry — skip */
      }
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
export function resolveAgentBinary(provider: Exclude<AgentProvider, 'claude'>, customBin?: string): string {
  if (customBin?.trim()) return customBin.trim();
  // Binary name matches the provider id for every managed CLI (codex/opencode/pi).
  return findOnPath(binNames(provider)) ?? provider;
}

/**
 * True if the provider's CLI is accessible right now. When `customBin` is set
 * we check that path directly; otherwise we search PATH.  PATH is read fresh,
 * so a just-installed CLI is detected without a restart.
 */
export function isAgentBinaryInstalled(provider: Exclude<AgentProvider, 'claude'>, customBin?: string): boolean {
  if (customBin?.trim()) {
    try { return fs.existsSync(customBin.trim()); } catch { return false; }
  }
  return findOnPath(binNames(provider)) !== null;
}

/** Check detection status for all providers (including Claude). `binaries` maps
 *  provider id → user-configured override path ('' = not set). */
export function checkAllProviders(
  binaries: Partial<Record<AgentProvider, string>> = {},
): ProviderStatus[] {
  const all: AgentProvider[] = ['claude', 'codex', 'opencode', 'pi'];
  return all.map((provider) => {
    const customBin = (binaries[provider] ?? '').trim();
    if (customBin) {
      let found = false;
      try { found = fs.existsSync(customBin); } catch {}
      return { provider, found, resolvedPath: found ? customBin : null, customBin };
    }
    const resolvedPath = findOnPath(binNames(provider));
    return { provider, found: resolvedPath !== null, resolvedPath, customBin: '' };
  });
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
