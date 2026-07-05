/**
 * Resolve how to launch the Claude CLI for the current platform.
 * Returns argv that claudemon's `/sessions/spawn` can execute.
 *
 * On Windows we want to match whatever `claude` the user runs in their own
 * terminal, so we resolve the `claude` shim on PATH first and launch its
 * package directly. Only if that fails do we scan nvm installs as a fallback.
 *
 * Two package layouts exist in the wild:
 *   - modern (>= 2.1.9x): a native launcher at `bin/claude.exe` — run directly
 *   - older:              a JS entrypoint at `cli.js`           — run via `node cli.js`
 */

import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';

const PKG_NAMES = ['@anthropic-ai/claude-code', 'claude-code'];

/** Given a claude-code package dir, return argv to launch it, or null if absent. */
function entrypointArgv(pkgDir: string, nodeExe?: string): string[] | null {
  // Modern layout ships a native launcher; prefer it (same exe the npm shim calls).
  const nativeExe = path.join(pkgDir, 'bin', 'claude.exe');
  if (fs.existsSync(nativeExe)) return [nativeExe];
  // Older layout: a JS entrypoint we run through node.
  const cliJs = path.join(pkgDir, 'cli.js');
  if (fs.existsSync(cliJs)) {
    return nodeExe && fs.existsSync(nodeExe) ? [nodeExe, cliJs] : ['node', cliJs];
  }
  return null;
}

function readPkgVersion(pkgDir: string): string {
  try {
    const pj = JSON.parse(fs.readFileSync(path.join(pkgDir, 'package.json'), 'utf8'));
    return typeof pj.version === 'string' ? pj.version : '0.0.0';
  } catch {
    return '0.0.0';
  }
}

/** Numeric semver compare (no prerelease handling — sufficient for claude-code). */
function compareVersions(a: string, b: string): number {
  const pa = a.split('.').map((n) => parseInt(n, 10) || 0);
  const pb = b.split('.').map((n) => parseInt(n, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] || 0) - (pb[i] || 0);
    if (d) return d;
  }
  return 0;
}

/**
 * Prefer the `claude` the user already has on PATH — that's the version their
 * terminal uses, and what they expect inside Workspacer. npm global installs
 * place the shim and its `node_modules` in the same prefix dir, so we resolve
 * the package entrypoint sitting next to the shim.
 */
function findClaudeOnPath(): { argv: string[] } | null {
  const dirs = (process.env.PATH || '').split(path.delimiter).filter(Boolean);
  const shims = process.platform === 'win32' ? ['claude.cmd', 'claude.exe', 'claude'] : ['claude'];
  const nodeName = process.platform === 'win32' ? 'node.exe' : 'node';
  for (const dir of dirs) {
    if (!shims.some((s) => fs.existsSync(path.join(dir, s)))) continue;
    const nodeExe = path.join(dir, nodeName);
    for (const pkg of PKG_NAMES) {
      const argv = entrypointArgv(path.join(dir, 'node_modules', pkg), nodeExe);
      if (argv) return { argv };
    }
  }
  return null;
}

/**
 * Fallback for Windows when `claude` isn't resolvable on PATH: scan nvm node
 * installs and pick the install with the highest claude-code *version* (not the
 * highest node version — those are unrelated).
 */
function findClaudeViaNvm(): { argv: string[] } | null {
  const nvmDir = path.join(os.homedir(), 'AppData', 'Local', 'nvm');
  let best: { version: string; argv: string[] } | null = null;
  try {
    for (const v of fs.readdirSync(nvmDir).filter((d) => d.startsWith('v'))) {
      const vDir = path.join(nvmDir, v);
      const node = path.join(vDir, 'node.exe');
      for (const pkg of PKG_NAMES) {
        const pkgDir = path.join(vDir, 'node_modules', pkg);
        const argv = entrypointArgv(pkgDir, node);
        if (!argv) continue;
        const version = readPkgVersion(pkgDir);
        if (!best || compareVersions(version, best.version) > 0) {
          best = { version, argv };
        }
      }
    }
  } catch {}
  return best ? { argv: best.argv } : null;
}

let cached: { argv: string[] } | null | undefined;
function getBaseArgv(): string[] {
  if (cached === undefined) {
    if (process.platform === 'win32') {
      cached = findClaudeOnPath() ?? findClaudeViaNvm();
    } else {
      cached = findClaudeOnPath() ?? { argv: ['claude'] };
    }
    if (cached) console.log(`[claudeResolver] base argv: ${cached.argv.join(' ')}`);
  }
  if (cached) return [...cached.argv];
  // Last-resort fallback on Windows (relies on the npm shim being on PATH).
  return process.platform === 'win32' ? ['cmd.exe', '/c', 'claude'] : ['claude'];
}

/** The resolved `claude` launcher argv with no session flags — e.g. `['claude']`
 *  or `['cmd.exe', '/c', 'claude']` on Windows. Handed to claudemon so it can
 *  run a headless `claude -p` summary call without re-resolving the binary. */
export function claudeBaseArgv(): string[] {
  return getBaseArgv();
}

export interface ClaudeArgvOptions {
  extraArgs?: string[];
  resumeSessionId?: string;
  /** Alias ('opus'/'sonnet') or full id ('claude-opus-4-8'). '' = Claude default. */
  model?: string;
  /** Pass `--dangerously-skip-permissions` to bypass all permission checks. */
  skipPermissions?: boolean;
  /**
   * Claude Code permission mode (`--permission-mode default|acceptEdits|plan`).
   * 'bypassPermissions' is expressed as `--dangerously-skip-permissions`
   * instead (the CLI treats them equivalently but the flag predates the mode).
   */
  permissionMode?: 'default' | 'acceptEdits' | 'plan' | 'bypassPermissions';
  /**
   * Pin the session id via `--session-id <uuid>` for a *new* session. Makes
   * claude name its transcript `<uuid>.jsonl`, so the id we track, claude's id,
   * and the transcript file all agree — no cwd-based guessing. Ignored when
   * resuming (the resumed id already fixes the file).
   */
  sessionId?: string;
  /** Absolute path to an MCP config JSON file (`--mcp-config <path>`). */
  mcpConfig?: string;
  /**
   * Pass `--strict-mcp-config` so the session sees ONLY the servers in
   * `mcpConfig`, ignoring the user's global `~/.claude.json` servers. Used for
   * per-spawn server selection where "exactly these" is the intent.
   */
  strictMcpConfig?: boolean;
  /**
   * Comma-joined tool glob(s) to pre-allow without a permission prompt.
   * Each entry is passed as a single comma-separated value to `--allowedTools`.
   * Example: `['mcp__workspacer']` → `--allowedTools mcp__workspacer`.
   */
  allowedTools?: string[];
  /**
   * Text appended to claude's system prompt via `--append-system-prompt`.
   * Useful for injecting role instructions into a supervisor session without
   * overwriting whatever the profile or user already configured.
   */
  appendSystemPrompt?: string;
}

export function buildClaudeArgv(opts: ClaudeArgvOptions = {}): string[] {
  const argv = getBaseArgv();
  if (opts.extraArgs && opts.extraArgs.length) argv.push(...opts.extraArgs);
  // Inject --model unless the profile's extraArgs already pin one.
  const profilePinsModel = (opts.extraArgs ?? []).some(
    (a) => a === '--model' || a.startsWith('--model='),
  );
  if (opts.model && opts.model.trim() && !profilePinsModel) {
    argv.push('--model', opts.model.trim());
  }
  // Inject --dangerously-skip-permissions unless a profile already set it.
  // 'bypassPermissions' mode rides the same flag.
  const alreadySkips = (opts.extraArgs ?? []).includes('--dangerously-skip-permissions');
  const wantsBypass = opts.skipPermissions || opts.permissionMode === 'bypassPermissions';
  if (wantsBypass && !alreadySkips) {
    argv.push('--dangerously-skip-permissions');
  }
  // Non-bypass permission modes map to --permission-mode, unless a profile
  // already pins one (or bypass is in play — the flags would fight).
  const profilePinsMode = (opts.extraArgs ?? []).some(
    (a) => a === '--permission-mode' || a.startsWith('--permission-mode='),
  );
  if (
    opts.permissionMode &&
    opts.permissionMode !== 'bypassPermissions' &&
    opts.permissionMode !== 'default' &&
    !wantsBypass &&
    !profilePinsMode
  ) {
    argv.push('--permission-mode', opts.permissionMode);
  }
  // Supervisor / MCP extras — appended after profile args so they always land.
  if (opts.mcpConfig) {
    argv.push('--mcp-config', opts.mcpConfig);
    if (opts.strictMcpConfig) argv.push('--strict-mcp-config');
  }
  if (opts.allowedTools && opts.allowedTools.length) {
    argv.push('--allowedTools', opts.allowedTools.join(','));
  }
  if (opts.appendSystemPrompt) {
    argv.push('--append-system-prompt', opts.appendSystemPrompt);
  }
  if (opts.resumeSessionId) {
    argv.push('--resume', opts.resumeSessionId);
  } else if (opts.sessionId) {
    // Pin the id for a fresh session, unless a profile already set one.
    const pinsId = (opts.extraArgs ?? []).some(
      (a) => a === '--session-id' || a.startsWith('--session-id='),
    );
    if (!pinsId) argv.push('--session-id', opts.sessionId);
  }
  return argv;
}
