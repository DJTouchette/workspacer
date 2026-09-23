import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  parseCleanupArgs,
  cleanupConfigPath,
  resolveCleanupOptions,
} from './cleanup-agent-artifacts.mjs';

test('cleanup CLI is dry-run unless --apply is explicit', () => {
  assert.equal(parseCleanupArgs([]).apply, false);
  assert.equal(parseCleanupArgs(['--apply']).apply, true);
  assert.throws(() => parseCleanupArgs(['--force']), /Unknown/);
  assert.throws(() => parseCleanupArgs(['--root']), /requires a value/);
  for (const value of ['-1', 'NaN', 'Infinity'])
    assert.throws(() => parseCleanupArgs(['--min-age-hours', value]), /nonnegative/);
});
test('CLI honors configured root/age and explicit overrides', () => {
  const config = {
    agents: { worktreeRoot: '/configured/worktrees', artifactCleanup: { minAgeHours: 8 } },
  };
  assert.deepEqual(resolveCleanupOptions(parseCleanupArgs([]), config, '/home/test'), {
    root: '/configured/worktrees',
    apply: false,
    minAgeMs: 8 * 3600000,
  });
  assert.deepEqual(
    resolveCleanupOptions(
      parseCleanupArgs(['--root', '/override', '--min-age-hours', '2', '--apply']),
      config,
    ),
    {
      root: '/override',
      apply: true,
      minAgeMs: 2 * 3600000,
    },
  );
  assert.equal(
    resolveCleanupOptions(parseCleanupArgs([]), {}, '/home/test').root,
    '/home/test/.workspacer/worktrees',
  );
  assert.throws(
    () =>
      resolveCleanupOptions(parseCleanupArgs([]), {
        agents: { artifactCleanup: { minAgeHours: '1' } },
      }),
    /nonnegative/,
  );
});
test('CLI follows configService config-path conventions', () => {
  assert.equal(
    cleanupConfigPath({}, 'linux', '/home/test'),
    '/home/test/.config/workspacer/config.yaml',
  );
  assert.equal(
    cleanupConfigPath({ XDG_CONFIG_HOME: '/config' }, 'linux', '/home/test'),
    '/config/workspacer/config.yaml',
  );
  assert.equal(
    cleanupConfigPath({ APPDATA: '/appdata' }, 'win32', '/home/test'),
    '/appdata/workspacer/config.yaml',
  );
});

test('CLI bundles the production core and dry-runs an isolated empty root', () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-cleanup-cli-test-'));
  try {
    const root = path.join(temp, 'worktrees');
    fs.mkdirSync(root);
    const output = execFileSync(
      process.execPath,
      [fileURLToPath(new URL('./cleanup-agent-artifacts.mjs', import.meta.url)), '--root', root],
      { encoding: 'utf8', env: { ...process.env, XDG_CONFIG_HOME: temp, APPDATA: temp } },
    );
    const report = JSON.parse(output);
    assert.equal(report.apply, false);
    assert.equal(report.root, fs.realpathSync(root));
    assert.equal(report.removedBytes, 0);
    assert.deepEqual(report.artifacts, []);
    assert.equal(fs.existsSync(root), true);
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
});
