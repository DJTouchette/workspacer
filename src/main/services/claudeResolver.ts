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
  const pa = a.split('.').map(n => parseInt(n, 10) || 0);
  const pb = b.split('.').map(n => parseInt(n, 10) || 0);
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
  const shims = process.platform === 'win32'
    ? ['claude.cmd', 'claude.exe', 'claude']
    : ['claude'];
  const nodeName = process.platform === 'win32' ? 'node.exe' : 'node';
  for (const dir of dirs) {
    if (!shims.some(s => fs.existsSync(path.join(dir, s)))) continue;
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
    for (const v of fs.readdirSync(nvmDir).filter(d => d.startsWith('v'))) {
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

export interface ClaudeArgvOptions {
  extraArgs?: string[];
  resumeSessionId?: string;
}

export function buildClaudeArgv(opts: ClaudeArgvOptions = {}): string[] {
  const argv = getBaseArgv();
  if (opts.extraArgs && opts.extraArgs.length) argv.push(...opts.extraArgs);
  if (opts.resumeSessionId) argv.push('--resume', opts.resumeSessionId);
  return argv;
}
