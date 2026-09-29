#!/usr/bin/env node
// Consume actual producer bytes from:
// cargo test --manifest-path services/hub-rs/Cargo.toml --test provider_parity \
//   actual_saved_timestamps_are_quoted_and_roundtrip -- --nocapture > /tmp/store-parity.log
// node apps/desktop/scripts/check-store-yaml-parity.mjs /tmp/store-parity.log
import assert from 'node:assert/strict';
import fs from 'node:fs';
import yaml from 'js-yaml';

if (process.argv.length !== 3) throw new Error('expected the Rust provider_parity test log path');
const marker = 'WKS_STORE_YAML_PARITY=';
const records = fs
  .readFileSync(process.argv[2], 'utf8')
  .split(/\r?\n/)
  .filter((line) => line.includes(marker))
  .map((line) => JSON.parse(line.slice(line.indexOf(marker) + marker.length)));
assert.equal(records.length, 2, 'must consume both actual Rust store outputs');
assert.deepEqual(records.map((record) => record.store).sort(), ['layouts', 'sessions']);
for (const record of records) {
  const field = record.store === 'layouts' ? 'createdAt' : 'timestamp';
  const decoded = yaml.load(record.yaml);
  assert.equal(
    typeof decoded[field],
    'string',
    `${record.store}: YAML must preserve string typing`,
  );
  assert.equal(decoded[field], record.document[field]);
  assert.deepEqual(
    decoded,
    record.document,
    `${record.store}: desktop and Rust must load the same saved bytes`,
  );
}
assert.ok(
  yaml.load('timestamp: 2026-03-01T00:00:00.000Z\n').timestamp instanceof Date,
  'control: the default desktop loader must still distinguish implicit YAML timestamps',
);
const matrixMarker = 'WKS_STORE_TIMESTAMP_MATRIX=';
const matrices = fs
  .readFileSync(process.argv[2], 'utf8')
  .split(/\r?\n/)
  .filter((line) => line.includes(matrixMarker))
  .map((line) => JSON.parse(line.slice(line.indexOf(matrixMarker) + matrixMarker.length)));
assert.equal(matrices.length, 1, 'actual timestamp matrix must be present exactly once');
const matrix = matrices[0];
const required = [
  'canonical',
  'date-only',
  'short-valid-fields',
  'rejected-short-time',
  'rejected-short-date',
  'space-time',
  'short-offset',
  'colon-offset',
  'fraction',
  'empty-fraction',
  'year-zero',
  'year-99',
  'year-100',
  'extended-year-rollover',
  'calendar-rollover',
  'ordinary-text',
  'quoted-offset',
  'explicit-string',
  'explicit-time',
  'directive-string',
  'directive-time',
  'plain-anchor',
  'quoted-anchor',
  'number',
  'null',
  'mapping',
  'array',
  'invalid-explicit-time',
  'invalid-explicit-short-date',
  'flow-map',
  'offset-whitespace',
  'rejected-offset',
  'uri-time',
  'uri-string',
];
assert.equal(
  new Set(matrix.map((row) => `${row.store}/${row.case}`)).size,
  matrix.length,
  'duplicate matrix rows',
);
for (const store of ['layouts', 'sessions'])
  for (const name of required)
    assert.ok(
      matrix.some((row) => row.store === store && row.case === name),
      `missing ${store}/${name}`,
    );
for (const row of matrix) {
  const field = row.store === 'layouts' ? 'createdAt' : 'timestamp';
  let decoded;
  try {
    decoded = yaml.load(row.yaml);
  } catch {
    assert.equal(
      row.present,
      false,
      `${row.store}/${row.case}: invalid explicit scalar was admitted`,
    );
    continue;
  }
  assert.equal(row.present, true, `${row.store}/${row.case}: valid row was lost`);
  const value = decoded[field];
  const date = value instanceof Date && Number.isFinite(value.getTime());
  const expected = date
    ? value.toISOString()
    : row.store === 'layouts'
      ? value
      : typeof value === 'string'
        ? value
        : '';
  assert.deepEqual(row.projection, expected, `${row.store}/${row.case}: timestamp resolver drift`);
}
console.log(
  `Validated two actual Rust saves and ${matrix.length} timestamp projections with desktop js-yaml.`,
);
