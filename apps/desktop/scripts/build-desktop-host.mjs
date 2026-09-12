#!/usr/bin/env node
import fs from 'node:fs';
import { build } from 'esbuild';
import './gen-desktop-services.mjs';
await build({
  entryPoints: ['src/main/headless/stdio.ts'],
  outfile: 'dist/headless/desktop-host.cjs',
  bundle: true, platform: 'node', format: 'cjs', target: 'node22',
  banner: { js: 'console.log = (...args) => console.error(...args);' },
  plugins: [{ name: 'no-electron-in-headless', setup(b) {
    b.onResolve({ filter: /^electron$/ }, () => ({ errors: [{ text: 'Headless services must not depend on Electron; inject their runtime input instead.' }] }));
  } }],
});

fs.copyFileSync('dist/headless/desktop-host.cjs', '../../services/hub/desktop-host.cjs');
