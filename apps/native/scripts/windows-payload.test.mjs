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
    'services/hub-rs/target/release/workspacer-rust.exe',
    ...['workspacer', 'hub', 'brain', 'mcp'].map(name => `services/hub/${name}.exe`),
    'apps/desktop/dist/headless/desktop-host.cjs', 'node/node.exe', 'node/LICENSE',
    'LICENSE', 'apps/native/packaging/windows/README.txt',
    'crt/vcruntime140.dll', 'crt/msvcp140.dll', 'crt/vcruntime140_1.dll',
    'plugins/examples/sample/manifest.json',
  ];
  for (const file of files) {
    const target = path.join(root, file);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, file);
  }
  fs.mkdirSync(path.join(root, 'apps/native/packaging/icons'), { recursive: true });
  fs.copyFileSync(path.join(repo, 'apps/native/packaging/icons/icon.ico'), path.join(root, 'apps/native/packaging/icons/icon.ico'));
  return {
    root, stage: path.join(root, 'apps/native/target/windows-package'),
    crt: path.join(root, 'crt'), nodeExecutable: path.join(root, 'node/node.exe'),
    version: '0.169.0-nightly.202609271234', commit: 'fixture-sha', nodeVersion: 'v22.20.0',
  };
}

test('default payload includes Rust backend with a conservative uninstall manifest', t => {
  const options = fixture(t);
  const { stage, uninstall } = stagePayload(options);
  for (const file of ['wks-native.exe', 'workspacer-rust.exe', 'README.txt', 'icon.ico', 'vcruntime140.dll', 'vcruntime140_1.dll',
    'msvcp140.dll', 'examples/sample/manifest.json']) assert.ok(fs.statSync(path.join(stage, file)).isFile(), file);
  const stamp = JSON.parse(fs.readFileSync(path.join(stage, 'build-stamp.json')));
  assert.equal(stamp.version, options.version);
  assert.equal(stamp.commit, 'fixture-sha');
  const removal = fs.readFileSync(uninstall, 'utf8');
  assert.match(removal, /Delete "\$INSTDIR\\workspacer-rust\.exe"/);
  assert.match(removal, /Delete "\$INSTDIR\\examples\\sample\\manifest\.json"/);
  assert.doesNotMatch(removal, /RMDir \/r|Delete .*\*|APPDATA/i);
  // Rebuilding must not sweep stale files into the next installer.
  fs.writeFileSync(path.join(stage, 'stale.exe'), 'old');
  stagePayload(options);
  assert.ok(!fs.existsSync(path.join(stage, 'stale.exe')));
});

test('missing runtime or service makes staging fail', t => {
  for (const file of ['services/hub-rs/target/release/workspacer-rust.exe', 'apps/native/target/release/wks-native.exe', 'crt/vcruntime140.dll']) {
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

test('payload needs no Go or Node input and excludes every legacy companion', t => {
  const options = { ...fixture(t), backend: 'rust', nodeExecutable: undefined, nodeVersion: undefined };
  for (const name of ['workspacer', 'hub', 'brain', 'mcp']) fs.unlinkSync(path.join(options.root, `services/hub/${name}.exe`));
  fs.rmSync(path.join(options.root, 'node'), { recursive: true });
  fs.rmSync(path.join(options.root, 'apps/desktop/dist'), { recursive: true });
  const { stage, uninstall } = stagePayload(options);
  assert.deepEqual(fs.readdirSync(stage).filter(f => f.endsWith('.exe')).sort(), ['wks-native.exe', 'workspacer-rust.exe']);
  assert.ok(!fs.existsSync(path.join(stage, 'desktop-host.cjs')));
  assert.ok(!fs.existsSync(path.join(stage, 'NODE-LICENSE.txt')));
  const stamp = JSON.parse(fs.readFileSync(path.join(stage, 'build-stamp.json')));
  assert.equal(stamp.backend, 'rust');
  assert.ok(!('node' in stamp));
  assert.match(fs.readFileSync(uninstall, 'utf8'), /workspacer-rust\.exe/);
  fs.unlinkSync(path.join(options.root, 'services/hub-rs/target/release/workspacer-rust.exe'));
  assert.throws(() => stagePayload(options), /ENOENT/);
});

test('unknown backend is refused before changing an existing stage', t => {
  const options=fixture(t); fs.mkdirSync(options.stage,{recursive:true});
  fs.writeFileSync(path.join(options.stage,'keep.txt'),'safe');
  assert.throws(()=>stagePayload({...options,backend:'typo'}),/Unknown/);
  assert.equal(fs.readFileSync(path.join(options.stage,'keep.txt'),'utf8'),'safe');
});

test('installer has one product identity and retires the preview one without touching user data', () => {
  const script = fs.readFileSync(path.join(repo, 'apps/native/packaging/windows/installer.nsi'), 'utf8');
  assert.match(script, /^!define PRODUCT "Workspacer Native"$/m);
  assert.match(script, /^!define APP_ID "Workspacer\.Native"$/m);
  assert.doesNotMatch(script, /RUST_PREVIEW/);
  // In-place upgrade from the preview: its folder becomes the default, and
  // its registration goes after a successful install.
  assert.match(script, /^!define LEGACY_PRODUCT "Workspacer Native Rust Preview"$/m);
  assert.match(script, /ReadRegStr \$0 HKCU "Software\\\$\{LEGACY_PRODUCT\}" "InstallDir"/);
  assert.match(script, /DeleteRegKey HKCU "\$\{LEGACY_UNINSTALL_KEY\}"/);
  assert.match(script, /DeleteRegKey HKCU "Software\\\$\{LEGACY_PRODUCT\}"/);
  // Never the old uninstaller, never a recursive delete, never the data folders.
  const code = script.split('\n').filter(line => !line.trim().startsWith(';')).join('\n');
  assert.doesNotMatch(code, /ExecWait|Exec |RMDir \/r|Delete .*\*|LOCALAPPDATA\\\$\{(LEGACY_)?PRODUCT\}/);
  assert.equal([...code.matchAll(/LOCALAPPDATA/g)].length, 1, 'only the default install folder names LOCALAPPDATA');
});
