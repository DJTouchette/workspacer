#!/usr/bin/env node
import fs from 'node:fs';
import { build } from 'esbuild';
await build({
  entryPoints: ['src/main/services/intentFileWorker.ts'],
  outfile: 'dist/headless/intent-file-worker.cjs',
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node22',
  plugins: [
    {
      name: 'no-electron-in-intent-worker',
      setup(builder) {
        builder.onResolve({ filter: /^electron$/ }, () => ({
          errors: [{ text: 'Intent file worker must be independent of Electron.' }],
        }));
      },
    },
  ],
});
fs.copyFileSync(
  'dist/headless/intent-file-worker.cjs',
  '../../services/hub/intent-file-worker.cjs',
);
