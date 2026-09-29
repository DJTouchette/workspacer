/** Build the opt-in Rust contract harness, never a Go/private Node server. */
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs';
const REPO = path.resolve(__dirname, '../../../../..');
const BUILD_HOME = os.homedir();
let compiledFixture: string | undefined;
export function buildRustHubFixture(): string {
  if (compiledFixture) return compiledFixture;
  const prebuilt = process.env.WKS_TEST_HUB_FIXTURE_BINARY;
  if (prebuilt) {
    if (!path.isAbsolute(prebuilt) || !fs.statSync(prebuilt).isFile())
      throw new Error('WKS_TEST_HUB_FIXTURE_BINARY must name an absolute fixture executable');
    fs.accessSync(prebuilt, process.platform === 'win32' ? fs.constants.F_OK : fs.constants.X_OK);
    compiledFixture = fs.realpathSync(prebuilt);
    return compiledFixture;
  }
  // Runtime credentials and account variables do not enter the build child.
  const env: NodeJS.ProcessEnv = {};
  for (const key of [
    'PATH',
    'SystemRoot',
    'SYSTEMROOT',
    'TEMP',
    'TMP',
    'TMPDIR',
    'CARGO_HOME',
    'RUSTUP_HOME',
    'CARGO_TARGET_DIR',
    'RUSTFLAGS',
    'CC',
    'CXX',
    'AR',
    'LD_LIBRARY_PATH',
  ]) {
    if (process.env[key] !== undefined) env[key] = process.env[key];
  }
  env.HOME = BUILD_HOME;
  env.USERPROFILE = BUILD_HOME;
  env.CARGO_BUILD_JOBS = process.env.CARGO_BUILD_JOBS || '2';
  env.CARGO_INCREMENTAL = process.env.CARGO_INCREMENTAL || '0';
  const built = spawnSync(process.execPath, [path.join(__dirname, 'rustBuildProcess.cjs')], {
    input: JSON.stringify({
      command: 'cargo',
      parentPid: process.pid,
      timeoutMs: 600_000,
      args: [
        'build',
        '--locked',
        '--manifest-path',
        path.join(REPO, 'services/hub-rs/Cargo.toml'),
        '--features',
        'test-support',
        '--example',
        'hub_contract_fixture',
        '--message-format=json',
      ],
    }),
    cwd: REPO,
    env,
    encoding: 'utf8',
    maxBuffer: 32 * 1024 * 1024,
    timeout: 615_000,
  });
  if (built.error || built.status !== 0) {
    const diagnostics = built.stdout
      .split('\n')
      .flatMap((line) => {
        try {
          const message = JSON.parse(line);
          return message.reason === 'compiler-message' && message.message?.rendered
            ? [message.message.rendered]
            : [];
        } catch {
          return [];
        }
      })
      .join('\n');
    throw new Error(
      `Rust fixture build failed: ${built.error?.message || built.stderr}\n${diagnostics}`,
    );
  }
  for (const line of built.stdout.trim().split('\n').reverse()) {
    try {
      const item = JSON.parse(line);
      if (
        item.reason === 'compiler-artifact' &&
        item.target?.name === 'hub_contract_fixture' &&
        item.executable
      ) {
        compiledFixture = item.executable;
        return item.executable;
      }
    } catch {
      /* compiler output, not an artifact */
    }
  }
  throw new Error('Rust fixture compiler did not report its executable');
}
