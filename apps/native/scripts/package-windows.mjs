// Run after the native release build and the Rust backend build, on Windows x64.
// The release version is the root VERSION file; the installer lands in
// apps/native/target/package/ beside the Linux and macOS packages.
// NATIVE_CRT_DIR points at Visual Studio's redistributable x64 CRT directory.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { stagePayload } from './windows-payload.mjs';
import { resolveMakensis } from './resolve-nsis.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
if (process.platform !== 'win32' || process.arch !== 'x64') {
  throw new Error('Package on Windows x64 with a Windows x64 Node runtime');
}
const [nodeMajor, nodeMinor] = process.versions.node.split('.').map(Number);
if (nodeMajor !== 22 || nodeMinor < 13) throw new Error('Packaging requires Node 22.13 or newer in the Node 22 series');
const backend = process.env.NATIVE_BACKEND || 'rust';
if (backend !== 'rust') throw new Error('NATIVE_BACKEND must be rust');
const version = fs.readFileSync(path.join(root, 'VERSION'), 'utf8').trim();
if (!/^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/.test(version)) throw new Error('Invalid release version');
const stage = path.join(root, 'apps/native/target/windows-package');
const out = path.join(root, 'apps/native/target/package');
fs.mkdirSync(out, { recursive: true });
const { uninstall } = stagePayload({
  root, stage, crt: process.env.NATIVE_CRT_DIR,
  version, commit: process.env.GITHUB_SHA || null, backend,
});
const installer = path.join(out, `Workspacer-Setup-${version}-x64.exe`);
const nsis = await resolveMakensis();
if (!nsis) throw new Error('makensis not found: install NSIS, set MAKENSIS, or run npm ci in apps/desktop');
const compiler = nsis.path;
const result = spawnSync(compiler, [
  '/V3', '/WX', `/DVERSION=${version}`, `/DSTAGE=${stage}`, `/DOUTPUT=${installer}`,
  `/DUNINSTALL_FILES=${uninstall}`, path.join(root, 'apps/native/packaging/windows/installer.nsi'),
], { stdio: 'inherit', env: { ...process.env, ...(nsis.env?.NSISDIR ? { NSISDIR: nsis.env.NSISDIR } : {}) } });
if (result.error) throw result.error;
if (result.status !== 0) throw new Error(`NSIS exited ${result.status}`);
if (!fs.statSync(installer).size) throw new Error('Empty native installer');
console.log(installer);
