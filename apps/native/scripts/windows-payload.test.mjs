import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { stagePayload } from './windows-payload.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'native package '));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const files = [
    'apps/native/target/release/wks-native.exe',
    ...['workspacer', 'hub', 'brain', 'mcp'].map(name => `services/hub/${name}.exe`),
    'apps/desktop/dist/headless/desktop-host.cjs', 'node/node.exe', 'node/LICENSE',
    'LICENSE', 'apps/native/packaging/windows/README.txt',
    'crt/vcruntime140.dll', 'crt/msvcp140.dll', 'crt/vcruntime140_1.dll',
    'services/hub/examples/sample/manifest.json',
  ];
  for (const file of files) {
    const target = path.join(root, file);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, file);
  }
  fs.mkdirSync(path.join(root, 'apps/desktop/build'), { recursive: true });
  fs.copyFileSync(path.join(repo, 'apps/desktop/build/icon.ico'), path.join(root, 'apps/desktop/build/icon.ico'));
  return {
    root, stage: path.join(root, 'apps/native/target/windows-package'),
    crt: path.join(root, 'crt'), nodeExecutable: path.join(root, 'node/node.exe'),
    version: '0.169.0-nightly.202609271234', commit: 'fixture-sha', nodeVersion: 'v22.20.0',
  };
}

test('payload includes local services and private runtimes, with a conservative uninstall manifest', t => {
  const options = fixture(t);
  const { stage, uninstall } = stagePayload(options);
  for (const file of ['wks-native.exe', 'workspacer.exe', 'hub.exe', 'brain.exe', 'mcp.exe',
    'desktop-host.cjs', 'node.exe', 'NODE-LICENSE.txt', 'vcruntime140.dll', 'vcruntime140_1.dll',
    'msvcp140.dll', 'examples/sample/manifest.json']) assert.ok(fs.statSync(path.join(stage, file)).isFile(), file);
  const stamp = JSON.parse(fs.readFileSync(path.join(stage, 'build-stamp.json')));
  assert.equal(stamp.version, options.version);
  assert.equal(stamp.commit, 'fixture-sha');
  const removal = fs.readFileSync(uninstall, 'utf8');
  assert.match(removal, /Delete "\$INSTDIR\\node\.exe"/);
  assert.match(removal, /Delete "\$INSTDIR\\examples\\sample\\manifest\.json"/);
  assert.doesNotMatch(removal, /RMDir \/r|Delete .*\*|APPDATA/i);
  // Rebuilding must not sweep stale files into the next installer.
  fs.writeFileSync(path.join(stage, 'stale.exe'), 'old');
  stagePayload(options);
  assert.ok(!fs.existsSync(path.join(stage, 'stale.exe')));
});

test('missing runtime or service makes staging fail', t => {
  for (const file of ['services/hub/mcp.exe', 'node/LICENSE', 'crt/vcruntime140.dll']) {
    const options = fixture(t);
    fs.unlinkSync(path.join(options.root, file));
    assert.throws(() => stagePayload(options), /ENOENT/);
  }
});

test('NSIS compiles the staged payload into a Windows installer', { skip: !process.env.MAKENSIS }, t => {
  const options = fixture(t);
  const { stage, uninstall } = stagePayload(options);
  const output = path.join(options.root, 'Native Setup.exe');
  const prefix = process.platform === 'win32' ? '/' : '-';
  const args = ['V3', 'WX', `DVERSION=${options.version}`, `DSTAGE=${stage}`, `DOUTPUT=${output}`, `DUNINSTALL_FILES=${uninstall}`]
    .map(arg => prefix + arg);
  args.push(path.join(repo, 'apps/native/packaging/windows/installer.nsi'));
  const result = spawnSync(process.env.MAKENSIS, args, { encoding: 'utf8' });
  assert.equal(result.status, 0, `${result.error || ''}\n${result.stdout}\n${result.stderr}`);
  assert.equal(fs.readFileSync(output).subarray(0, 2).toString(), 'MZ');
});
