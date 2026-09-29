#!/usr/bin/env node
// Portable captured registration vocabulary; presence is not behavioral parity.
const fs = require('node:fs'), path = require('node:path');
const root = path.resolve(__dirname, '..');
const source = 'contracts/backend-capabilities.json';
const reference = JSON.parse(fs.readFileSync(path.join(root, source), 'utf8'));
const desktop = JSON.parse(fs.readFileSync(path.join(root, 'contracts/desktop-service-methods.json'), 'utf8'));
const sorted = values => [...new Set(values)].sort();
for (const scope of ['full', 'catalog', 'hub']) {
  const names = reference[scope];
  if (!Array.isArray(names) || names.some(name => typeof name !== 'string' || !/^[A-Za-z]+(?:\.[A-Za-z]+)+$/.test(name)))
    throw Error(`Invalid ${scope} capability contract`);
}
const spec = {
  source: `${source} + contracts/desktop-service-methods.json`,
  architecturalRetirements: reference.architecturalRetirements || {},
  hub: sorted(reference.hub),
  full: sorted([...reference.full, ...desktop.ownerMethods, ...desktop.assetMethods]),
  catalog: sorted(reference.catalog),
};
for (const [method, evidence] of Object.entries(spec.architecturalRetirements)) {
  if (!spec.hub.includes(method) || !evidence.reason?.trim()) throw Error(`Invalid retirement: ${method}`);
  for (const file of [evidence.replacement, evidence.tests]) {
    if (!file?.startsWith('services/hub-rs/') || !fs.statSync(path.join(root, file)).isFile())
      throw Error(`Missing replacement evidence: ${method}`);
  }
}
if (spec.full.length < 100 || spec.catalog.length < 20 || spec.hub.length < 35)
  throw Error('Reference registry unexpectedly incomplete');
const output = JSON.stringify(spec, null, 2) + '\n';
const target = path.join(root, 'services/hub-rs/assets/brain-capabilities.json');
if (process.argv.includes('--check')) {
  if (fs.readFileSync(target, 'utf8') !== output) throw Error('Rust capability reference is stale');
} else fs.writeFileSync(target, output);
