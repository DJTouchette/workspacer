#!/usr/bin/env node
// tsc does not delete emitted files when their source disappears. Never package
// stale private-companion transport after an incremental Electron build.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
export function prepareMainOutput(desktop) {
  fs.rmSync(path.join(desktop, 'dist/main'), { recursive: true, force: true });
  fs.rmSync(path.join(desktop, 'dist/headless/desktop-host.cjs'), { force: true });
  fs.rmSync(path.join(desktop, 'dist/headless/desktop-host.cjs.map'), { force: true });
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  prepareMainOutput(path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..'));
