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

export type AgentProvider = 'claude' | 'codex' | 'opencode';

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
 * Resolve the launcher binary for a provider. Falls back to the bare command
 * name (relying on the user's PATH at spawn time) when not found up front, so a
 * freshly-installed CLI still works without a restart.
 */
export function resolveAgentBinary(provider: Exclude<AgentProvider, 'claude'>): string {
  const base = provider === 'codex' ? 'codex' : 'opencode';
  return findOnPath(binNames(base)) ?? base;
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
