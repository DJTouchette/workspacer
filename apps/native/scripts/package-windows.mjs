// Run after the native release build and desktop/service builds, on Windows x64.
// NATIVE_CRT_DIR points at Visual Studio's redistributable x64 CRT directory.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { stagePayload } from './windows-payload.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
if (process.platform !== 'win32' || process.arch !== 'x64') {
  throw new Error('Package on Windows x64 with a Windows x64 Node runtime');
}
const [nodeMajor, nodeMinor] = process.versions.node.split('.').map(Number);
if (nodeMajor !== 22 || nodeMinor < 13) throw new Error('Packaging requires Node 22.13 or newer in the Node 22 series');
const version = JSON.parse(fs.readFileSync(path.join(root, 'apps/desktop/package.json'))).version;
if (!/^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/.test(version)) throw new Error('Invalid release version');
const stage = path.join(root, 'apps/native/target/windows-package');
const out = path.join(root, 'apps/desktop/release');
fs.mkdirSync(out, { recursive: true });
const { uninstall } = stagePayload({
  root, stage, crt: process.env.NATIVE_CRT_DIR, nodeExecutable: process.execPath,
  version, commit: process.env.GITHUB_SHA || null, nodeVersion: process.version,
});
const installer = path.join(out, `Workspacer-Native-Setup-${version}-x64.exe`);
const compiler = process.env.MAKENSIS || path.join(process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)', 'NSIS/makensis.exe');
const result = spawnSync(compiler, [
  '/V3', `/DVERSION=${version}`, `/DSTAGE=${stage}`, `/DOUTPUT=${installer}`,
  `/DUNINSTALL_FILES=${uninstall}`, path.join(root, 'apps/native/packaging/windows/installer.nsi'),
], { stdio: 'inherit' });
if (result.error) throw result.error;
if (result.status !== 0) throw new Error(`NSIS exited ${result.status}`);
if (!fs.statSync(installer).size) throw new Error('Empty native installer');
console.log(installer);
