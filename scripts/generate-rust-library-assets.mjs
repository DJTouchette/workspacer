#!/usr/bin/env node
// Build-time only: bundle authoritative desktop starter data into the Rust host.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { Script } from 'node:vm';
const root = new URL('../', import.meta.url);
const source = readFileSync(new URL('apps/desktop/src/main/services/libraryService.ts', root), 'utf8');
const match = source.match(/private starters\(\): Array<[^\n]+> \{\s*return ([\s\S]*?);\n  \}/);
if (!match) throw new Error('Starter declaration changed; update Rust asset generator');
const output = new Script(`JSON.stringify((${match[1]}), null, 2)`).runInNewContext({}, { timeout: 1000 }) + '\n';
const target = new URL('services/hub-rs/assets/library-starters.json', root);
if (process.argv.includes('--check')) {
  if (readFileSync(target, 'utf8') !== output) throw new Error(`Stale asset: ${fileURLToPath(target)}`);
} else writeFileSync(target, output);
