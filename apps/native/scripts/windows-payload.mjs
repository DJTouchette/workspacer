import fs from 'node:fs';
import path from 'node:path';

// Stage only the explicit native runtime and generate a file-specific uninstall list.
export function stagePayload({ root, stage, crt, version, commit = null, backend = 'rust' }) {
  if (backend !== 'rust') throw new Error('Unknown native backend');
  fs.rmSync(stage, { recursive: true, force: true });
  fs.mkdirSync(stage, { recursive: true });
  const copy = (source, name = path.basename(source)) => {
    if (!fs.statSync(source).isFile()) throw new Error(`Missing package file: ${source}`);
    fs.copyFileSync(source, path.join(stage, name));
  };
  copy(path.join(root, 'apps/native/target/release/wks-native.exe'));
  copy(path.join(root, 'services/hub-rs/target/release/workspacer-rust.exe'));
  copy(path.join(root, 'LICENSE'), 'LICENSE.txt');
  copy(path.join(root, 'apps/desktop/build/icon.ico'));
  copy(path.join(root, 'apps/native/packaging/windows/README-rust.txt'), 'README.txt');
  if (!crt) throw new Error('NATIVE_CRT_DIR is required');
  for (const name of ['vcruntime140.dll', 'msvcp140.dll']) {
    if (!fs.statSync(path.join(crt, name)).isFile()) throw new Error(`Missing CRT: ${name}`);
  }
  for (const name of fs.readdirSync(crt).filter(name => name.endsWith('.dll'))) copy(path.join(crt, name));
  fs.cpSync(path.join(root, 'plugins/examples'), path.join(stage, 'examples'), { recursive: true });
  fs.writeFileSync(path.join(stage, 'build-stamp.json'), JSON.stringify({
    component: 'native', version, commit,
    platform: 'windows-x64', backend,
  }, null, 2) + '\n');

  // Enumerate owned files so uninstall never recursively deletes user-created files.
  const files = [], dirs = [];
  function walk(dir, relative = '') {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const name = path.join(relative, entry.name);
      if (entry.isDirectory()) { dirs.push(name); walk(path.join(dir, entry.name), name); }
      else if (entry.isFile()) files.push(name);
      else throw new Error(`Unsupported package entry: ${name}`);
    }
  }
  walk(stage);
  const nsisPath = name => name.replaceAll('$', '$$').replaceAll('"', '$\\"').replaceAll('/', '\\');
  const uninstall = path.join(path.dirname(stage), 'windows-uninstall.nsh');
  fs.writeFileSync(uninstall, [
    ...files.map(name => `ClearErrors
  Delete "$INSTDIR\\${nsisPath(name)}"
  \${If} \${Errors}
    SetErrorLevel 1
    Abort "Close Workspacer Native before uninstalling."
  \${EndIf}`),
    ...dirs.reverse().map(name => `RMDir "$INSTDIR\\${nsisPath(name)}"`),
  ].join('\n') + '\n');
  return { stage, uninstall };
}
