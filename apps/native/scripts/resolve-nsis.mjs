// Find the NSIS compiler (makensis) for the native Windows installer.
//
// Resolution order:
//   1. MAKENSIS: an explicit path, which must exist.
//   2. makensis on PATH, then the standard install folders
//      (%ProgramFiles(x86)%\NSIS, %ProgramFiles%\NSIS). GitHub's Windows
//      runners ship NSIS there, so CI needs no node_modules.
//   3. The copy electron-builder downloads under apps/desktop/node_modules,
//      for dev machines that ran `npm ci` in apps/desktop and have no NSIS.
//
// As a command it writes MAKENSIS (and NSISDIR, for the node_modules copy) to
// GitHub's environment file, or to the file named by argv[2]:
//   node apps/native/scripts/resolve-nsis.mjs [env-output-file]
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const isFile = candidate => {
  try { return fs.statSync(candidate).isFile(); } catch { return false; }
};

// Steps 1 and 2. Returns { path, source } or null; throws on a bad MAKENSIS.
export function findMakensis(env = process.env, platform = process.platform) {
  if (env.MAKENSIS) {
    if (!isFile(env.MAKENSIS)) throw new Error(`MAKENSIS does not name a file: ${env.MAKENSIS}`);
    return { path: env.MAKENSIS, source: 'MAKENSIS' };
  }
  const windows = platform === 'win32';
  const names = windows ? ['makensis.exe'] : ['makensis'];
  const searchPath = (windows ? env.Path || env.PATH : env.PATH) || '';
  for (const dir of searchPath.split(path.delimiter).filter(Boolean)) {
    for (const name of names) {
      const candidate = path.join(dir, name);
      if (isFile(candidate)) return { path: candidate, source: 'PATH' };
    }
  }
  if (windows) {
    const bases = [env['ProgramFiles(x86)'], env.ProgramFiles, 'C:\\Program Files (x86)', 'C:\\Program Files'];
    for (const base of bases.filter(Boolean)) {
      const candidate = path.join(base, 'NSIS', 'makensis.exe');
      if (isFile(candidate)) return { path: candidate, source: 'standard install folder' };
    }
  }
  return null;
}

// Step 3. Returns { path, source, env } or null when apps/desktop has no
// node_modules (or its electron-builder has no NSIS toolset).
export async function findDesktopMakensis() {
  let toolset;
  try {
    const requireDesktop = createRequire(path.join(root, 'apps/desktop/package.json'));
    toolset = requireDesktop('app-builder-lib/out/toolsets/windows.js');
  } catch {
    return null;
  }
  const compiler = await toolset.getMakeNsisPath();
  if (!compiler?.path || !isFile(compiler.path)) return null;
  return { path: compiler.path, source: 'apps/desktop/node_modules', env: compiler.env };
}

export async function resolveMakensis(env = process.env) {
  return findMakensis(env) || await findDesktopMakensis();
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const output = process.argv[2] || process.env.GITHUB_ENV;
  if (!output) throw new Error('Pass an environment output file or set GITHUB_ENV');
  const compiler = await resolveMakensis();
  if (!compiler) {
    throw new Error('makensis not found: install NSIS (https://nsis.sourceforge.io), set MAKENSIS, or run `npm ci` in apps/desktop');
  }
  const values = { MAKENSIS: compiler.path };
  if (compiler.env?.NSISDIR) values.NSISDIR = compiler.env.NSISDIR;
  for (const [key, value] of Object.entries(values)) {
    if (/[\r\n]/.test(value)) throw new Error(`Invalid ${key} path`);
    fs.appendFileSync(output, `${key}=${value}\n`);
  }
  console.log(`Native installer compiler (${compiler.source}): ${compiler.path}`);
  // Record the compiler version in the build log.
  const flag = process.platform === 'win32' ? '/VERSION' : '-VERSION';
  const probe = spawnSync(compiler.path, [flag], { encoding: 'utf8', env: { ...process.env, ...values } });
  if (probe.status !== 0) throw new Error(`makensis ${flag} failed: ${probe.error || probe.stderr || probe.status}`);
  console.log(`makensis ${flag}: ${probe.stdout.trim()}`);
}
