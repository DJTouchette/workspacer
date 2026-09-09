import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { createHash } from 'crypto';

/** Verified wrapper changes PATH only before forwarding to @openai/codex.
 * Bypass its npm/mise installation steps for readiness. Never interpret shell
 * syntax, execute npx, search arbitrary package names, or guess among versions.
 * The adapter still verifies the native CLI version before reading its config.
 */
const WRAPPER_SHA256 = '0f769462bfa40e84c92f1c4481b268ecaca740dda521338f7255406496aaf3e6';
export function resolveCodexReadinessBinary(configured: string): string | null {
  try {
    const stat = fs.statSync(configured);
    if (!stat.isFile()) return null;
    // Native files are handled by the adapter, not read wholesale here.
    if (stat.size > 8192) return configured;
    const content = fs.readFileSync(configured);
    if (!content.subarray(0, 2).equals(Buffer.from('#!'))) return configured;
    if (createHash('sha256').update(content).digest('hex') !== WRAPPER_SHA256) return null;
    const targets: Record<string, string> = {
      'linux-x64': 'x86_64-unknown-linux-musl',
      'linux-arm64': 'aarch64-unknown-linux-musl',
      'darwin-x64': 'x86_64-apple-darwin',
      'darwin-arm64': 'aarch64-apple-darwin',
    };
    const platform = `${process.platform}-${process.arch}`,
      target = targets[platform];
    if (!target) return null;
    const cache = path.join(
      process.env.npm_config_cache || path.join(os.homedir(), '.npm'),
      '_npx',
    );
    const entries = fs.readdirSync(cache);
    if (entries.length > 256) return null;
    const candidates: string[] = [];
    for (const entry of entries) {
      const modules = path.join(cache, entry, 'node_modules', '@openai');
      try {
        const pkg = JSON.parse(
          fs.readFileSync(path.join(modules, 'codex', 'package.json'), 'utf8'),
        );
        if (pkg.name !== '@openai/codex' || pkg.version !== '0.153.4') continue;
        const native = path.join(modules, `codex-${platform}`, 'vendor', target, 'bin', 'codex');
        if (fs.statSync(native).isFile()) candidates.push(fs.realpathSync(native));
      } catch {
        /* unrelated cache entry */
      }
    }
    const unique = [...new Set(candidates)];
    return unique.length === 1 ? unique[0] : null;
  } catch {
    return null;
  }
}
