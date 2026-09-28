// Reuse the checksum-verified compiler from the pinned desktop packaging toolchain.
// Run after npm ci in apps/desktop. Writes GitHub's environment file (or argv[2]).
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const requireDesktop = createRequire(path.join(root, 'apps/desktop/package.json'));
const { getMakeNsisPath } = requireDesktop('app-builder-lib/out/toolsets/windows.js');
const output = process.argv[2] || process.env.GITHUB_ENV;
if (!output) throw new Error('Pass an environment output file or set GITHUB_ENV');
const compiler = await getMakeNsisPath();
if (!fs.statSync(compiler.path).isFile()) throw new Error('NSIS compiler is missing');
const values = { MAKENSIS: compiler.path };
if (compiler.env?.NSISDIR) values.NSISDIR = compiler.env.NSISDIR;
for (const [key, value] of Object.entries(values)) {
  if (/[\r\n]/.test(value)) throw new Error(`Invalid ${key} path`);
  fs.appendFileSync(output, `${key}=${value}\n`);
}
console.log(`Native installer compiler: ${compiler.path}`);
