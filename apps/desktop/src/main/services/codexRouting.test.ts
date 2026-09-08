import { describe, it, expect } from 'vitest';
import { mkdtempSync, mkdirSync, writeFileSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';
import { codexProviderFromConfig, codexProbeCommand, readCodexProvider } from './codexRouting';

describe('Codex launch routing', () => {
  it('extracts custom routing without credentials or auth settings', () => {
    expect(
      codexProviderFromConfig({
        model_provider: 'corp',
        model_providers: {
          corp: {
            base_url: 'https://example.com/v1/',
            http_headers: { Authorization: 'secret' },
            env_key: 'TOKEN',
          },
        },
      }),
    ).toEqual({ id: 'corp', baseUrl: 'https://example.com/v1' });
  });
  it.each([
    { profile: 'legacy' },
    { model_provider: 'corp' },
    { openai_base_url: 'https://secret@example.com/v1' },
    { openai_base_url: 'https://example.com/?key=secret' },
  ])('refuses ambiguous or credential-bearing routing %#', (config) => {
    expect(() => codexProviderFromConfig(config)).toThrow();
  });
  it('resolves npm Windows shims without a shell', () => {
    const dir = mkdtempSync(join(tmpdir(), 'codex-shim-'));
    mkdirSync(join(dir, 'node_modules/@openai/codex/bin'), { recursive: true });
    writeFileSync(join(dir, 'node_modules/@openai/codex/bin/codex.js'), '');
    writeFileSync(join(dir, 'node.exe'), '');
    expect(codexProbeCommand(join(dir, 'codex.cmd'))).toEqual([
      join(dir, 'node.exe'),
      join(dir, 'node_modules/@openai/codex/bin/codex.js'),
    ]);
  });
  it('asks config/read with the launch cwd, account and config flags, without starting a turn', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'codex-routing-'));
    const script = join(dir, 'cli.cjs');
    writeFileSync(
      script,
      `const readline = require('readline');
      if (process.env.CODEX_HOME !== 'chosen-account' || !process.argv.includes('model_provider="corp"')) process.exit(3);
      readline.createInterface({input: process.stdin}).on('line', line => {
        const m = JSON.parse(line);
        if (m.method === 'initialize') console.log(JSON.stringify({id: m.id, result: {}}));
        else if (m.method === 'config/read' && m.params.cwd === process.cwd()) console.log(JSON.stringify({id: m.id, result: {config: {model_provider: 'corp', model_providers: {corp: {base_url: 'https://example.com/v1', http_headers: {Authorization: 'secret'}}}}}}));
        else if (m.method !== 'initialized') process.exit(4);
      });`,
    );
    await expect(
      readCodexProvider([process.execPath, script], dir, { CODEX_HOME: 'chosen-account' }, [
        '--dangerously-bypass-approvals-and-sandbox',
        '-c',
        'model_provider="corp"',
      ]),
    ).resolves.toEqual({ id: 'corp', baseUrl: 'https://example.com/v1' });
    await expect(
      readCodexProvider([process.execPath, script], dir, {}, ['-p', 'named']),
    ).rejects.toThrow('native preset');
  });
});
