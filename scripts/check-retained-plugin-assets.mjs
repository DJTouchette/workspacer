#!/usr/bin/env node
// Retained assets are explicit copies, not an arbitrary runtime-directory scan.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const manifest = JSON.parse(fs.readFileSync(path.join(root, 'plugins/examples.provenance.json'), 'utf8'));
assert.match(manifest.referenceCommit, /^[0-9a-f]{40}$/);
assert.equal(manifest.files.length, 13);
const destinations = new Set();
for (const row of [...manifest.files, manifest.routingFixture]) {
  assert(!destinations.has(row.destination)); destinations.add(row.destination);
  assert(row.source.startsWith('services/hub/'));
  assert(row.destination.startsWith('plugins/examples/') || row.destination === 'apps/desktop/tests/fixtures/routing-preferences-view.json');
  for (const name of [row.source, row.destination]) assert(!name.split('/').includes('..'));
  const hash = (file) => createHash('sha256').update(fs.readFileSync(path.join(root, file))).digest('hex');
  // Original bytes are optional only after the separately reviewed deletion.
  if (fs.existsSync(path.join(root, row.source))) assert.equal(hash(row.source), row.sha256, row.source);
  if (row.destinationSha256) assert(row.relocationReason?.trim(), 'relocated bytes need a reviewed reason');
  assert.equal(hash(row.destination), row.destinationSha256 ?? row.sha256, row.destination);
}
console.log('Retained plugin13 + routing fixture1 byte/provenance checks passed.');
