/**
 * Resolve how to launch the Claude CLI for the current platform.
 * Returns argv that claudemon's `/sessions/spawn` can execute.
 *
 * On Windows, `claude` isn't usually on PATH — it lives inside a node_modules
 * dir under whatever Node version the user has via nvm. We sniff that out
 * and produce `node <cli.js>` argv, falling back to `cmd.exe /c claude` if
 * we can't find it (which works if the user has the npm shim on PATH).
 */

import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';

function findClaudeOnWindows(): { argv: string[] } | null {
  const nvmDir = path.join(os.homedir(), 'AppData', 'Local', 'nvm');
  try {
    const versions = fs.readdirSync(nvmDir)
      .filter(d => d.startsWith('v'))
      .sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
    for (const v of versions) {
      const vDir = path.join(nvmDir, v);
      for (const pkg of ['@anthropic-ai/claude-code', 'claude-code']) {
        const script = path.join(vDir, 'node_modules', pkg, 'cli.js');
        if (fs.existsSync(script)) {
          const node = path.join(vDir, 'node.exe');
          if (fs.existsSync(node)) return { argv: [node, script] };
        }
      }
    }
  } catch {}
  return null;
}

let cached: { argv: string[] } | null | undefined;
function getBaseArgv(): string[] {
  if (cached === undefined) {
    if (process.platform === 'win32') {
      cached = findClaudeOnWindows();
    } else {
      cached = { argv: ['claude'] };
    }
    if (cached) console.log(`[claudeResolver] base argv: ${cached.argv.join(' ')}`);
  }
  if (cached) return [...cached.argv];
  // Last-resort fallback on Windows
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
