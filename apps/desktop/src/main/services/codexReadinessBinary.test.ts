import * as fs from 'fs';
import * as path from 'path';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { resolveCodexReadinessBinary } from './codexReadinessBinary';
let root: string, wrapper: string, native: string;
beforeEach(() => {
  root = fs.mkdtempSync(path.join(process.cwd(), '.codex-binary-test-'));
  vi.stubEnv('npm_config_cache', root);
  wrapper = path.join(root, 'codex');
  fs.copyFileSync(path.join(__dirname, 'fixtures/codex-npx-wrapper.sh'), wrapper);
  const modules = path.join(root, '_npx', 'fixture', 'node_modules', '@openai');
  fs.mkdirSync(path.join(modules, 'codex'), { recursive: true });
  fs.writeFileSync(
    path.join(modules, 'codex/package.json'),
    JSON.stringify({ name: '@openai/codex', version: '0.153.4' }),
  );
  const targets: Record<string, string> = {
    'linux-x64': 'x86_64-unknown-linux-musl',
    'linux-arm64': 'aarch64-unknown-linux-musl',
    'darwin-x64': 'x86_64-apple-darwin',
    'darwin-arm64': 'aarch64-apple-darwin',
  };
  native = path.join(
    modules,
    `codex-${process.platform}-${process.arch}`,
    'vendor',
    targets[`${process.platform}-${process.arch}`] ?? 'unsupported',
    'bin',
    'codex',
  );
  fs.mkdirSync(path.dirname(native), { recursive: true });
  fs.writeFileSync(native, Buffer.from('7f454c46', 'hex'));
});
afterEach(() => {
  vi.unstubAllEnvs();
  fs.rmSync(root, { recursive: true, force: true });
});
it.skipIf(process.platform === 'win32')(
  'resolves only the exact reviewed wrapper to its installed native package without execution',
  () => {
    expect(resolveCodexReadinessBinary(wrapper)).toBe(native);
    expect(resolveCodexReadinessBinary(native)).toBe(native);
  },
);
it('refuses wrapper edits that could change account environment', () => {
  fs.appendFileSync(wrapper, '\nexport CODEX_HOME=/different-account\n');
  expect(resolveCodexReadinessBinary(wrapper)).toBeNull();
});
it('never picks a missing package or another package version', () => {
  fs.writeFileSync(
    path.join(root, '_npx/fixture/node_modules/@openai/codex/package.json'),
    JSON.stringify({ name: '@openai/codex', version: '0.154.0' }),
  );
  expect(resolveCodexReadinessBinary(wrapper)).toBeNull();
});
