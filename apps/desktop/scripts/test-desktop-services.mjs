#!/usr/bin/env node
// Portable backend contracts plus Electron's retained shared implementations.
// No private Node companion, Go executable, or provider inference is launched.
import path from 'node:path';
import { accessSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const rustTargets = [
  'legacy_preservation',
  'ui_assets',
  'files',
  'analytics',
  'profiles',
  'local_spawn',
  'fleet_review',
  'manager_replacements',
  'worker_results',
  'briefs',
  'workflows',
];
const electronTests = [
  'src/main/services/analyticsHistoryContract.test.ts',
  'src/main/shared/desktopServices.test.ts',
  'src/main/services/claudeAccountSetup.test.ts',
  'src/main/services/claudeProfilesContract.test.ts',
  'src/main/services/claudeProfilesProviders.test.ts',
  'src/main/services/managerReplacementService.test.ts',
  'src/main/services/managerReplacement.integration.test.ts',
  'src/main/services/dispatchHistoryStore.test.ts',
  'src/main/services/fleetWorkflowRuntime.test.ts',
  'src/main/services/fleetWorkflowService.test.ts',
];
// Vitest permits unmatched positional filters; reject stale entries explicitly.
for (const test of electronTests) accessSync(path.join(desktop, test));
for (const target of rustTargets)
  accessSync(path.resolve(desktop, '../../services/hub-rs/tests', `${target}.rs`));

for (const [binary, args] of [
  [process.execPath, ['--test', 'scripts/prepare-main-build.test.mjs']],
  [
    'cargo',
    [
      'test',
      '--locked',
      '--manifest-path',
      '../../services/hub-rs/Cargo.toml',
      ...rustTargets.flatMap((target) => ['--test', target]),
    ],
  ],
  [
    process.execPath,
    ['node_modules/vitest/vitest.mjs', 'run', '--no-file-parallelism', ...electronTests],
  ],
]) {
  const result = spawnSync(binary, args, { cwd: desktop, env: process.env, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
