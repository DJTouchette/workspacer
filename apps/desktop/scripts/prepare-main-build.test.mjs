import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { prepareMainOutput } from './prepare-main-build.mjs';

test('incremental builds remove deleted companion emissions without touching source or other assets', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'workspacer-main-output-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const file of [
    'dist/main/headless/stdio.js',
    'dist/headless/desktop-host.cjs',
    'dist/headless/desktop-host.cjs.map',
    'src/main/headless/desktopHost.ts',
    'dist/web/index.html',
  ]) {
    fs.mkdirSync(path.dirname(path.join(root, file)), { recursive: true });
    fs.writeFileSync(path.join(root, file), file);
  }
  prepareMainOutput(root);
  prepareMainOutput(root);
  assert.equal(fs.existsSync(path.join(root, 'dist/main')), false);
  assert.equal(fs.existsSync(path.join(root, 'dist/headless/desktop-host.cjs')), false);
  assert.equal(fs.existsSync(path.join(root, 'dist/headless/desktop-host.cjs.map')), false);
  assert.equal(
    fs.readFileSync(path.join(root, 'src/main/headless/desktopHost.ts'), 'utf8'),
    'src/main/headless/desktopHost.ts',
  );
  assert.equal(
    fs.readFileSync(path.join(root, 'dist/web/index.html'), 'utf8'),
    'dist/web/index.html',
  );
});
