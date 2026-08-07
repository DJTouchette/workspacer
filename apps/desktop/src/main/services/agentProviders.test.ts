// PATH resolution is a PARITY surface, not just a local convenience.
//
// providers.go's own header calls itself "a faithful Go port of
// agentProviders.ts: the same PATH probing and config binary-override honoring,
// so a web client's Spawn dialog sees the same model catalog and per-provider
// detection dots the desktop does". A DIRECTORY named like a provider binary is
// where the two came apart: `fs.existsSync` says yes, `os.Stat(...).IsDir()`
// says no. The brain skipped it and kept scanning PATH; this side returned the
// directory, which then became argv[0] of every spawn for that provider, and
// providers.checkAll lit a green detection dot for a provider that cannot
// launch.
//
// Twin: TestFindOnPathSkipsADirectory in cmd/brain/providers_test.go.

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { resolveAgentBinary, isAgentBinaryInstalled, checkAllProviders } from './agentProviders';

let sandbox: string;
let bin1: string;
let bin2: string;
const realPath = process.env.PATH;

beforeEach(() => {
  sandbox = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-path-')));
  bin1 = path.join(sandbox, 'bin1');
  bin2 = path.join(sandbox, 'bin2');
  // bin1/codex is a DIRECTORY; bin2/codex is the real thing, later on PATH.
  fs.mkdirSync(path.join(bin1, 'codex'), { recursive: true });
  fs.mkdirSync(bin2, { recursive: true });
  fs.writeFileSync(path.join(bin2, 'codex'), '#!/bin/sh\necho real\n', { mode: 0o755 });
  process.env.PATH = [bin1, bin2].join(path.delimiter);
});

afterEach(() => {
  process.env.PATH = realPath;
  fs.rmSync(sandbox, { recursive: true, force: true });
});

describe('agentProviders PATH probing matches the Go brain', () => {
  it('skips a directory on PATH and keeps scanning', () => {
    expect(resolveAgentBinary('codex')).toBe(path.join(bin2, 'codex'));
    expect(isAgentBinaryInstalled('codex')).toBe(true);
    const codex = checkAllProviders().find((p) => p.provider === 'codex')!;
    expect(codex.found).toBe(true);
    expect(codex.resolvedPath).toBe(path.join(bin2, 'codex'));
  });

  it('does not accept a directory as a configured binary override', () => {
    const dir = path.join(bin1, 'codex');
    expect(isAgentBinaryInstalled('codex', dir)).toBe(false);
    const codex = checkAllProviders({ codex: dir }).find((p) => p.provider === 'codex')!;
    expect(codex.found).toBe(false);
    expect(codex.resolvedPath).toBeNull();
    expect(codex.customBin).toBe(dir);
  });

  it('the floor: a real override is honoured', () => {
    const real = path.join(bin2, 'codex');
    expect(isAgentBinaryInstalled('codex', real)).toBe(true);
    expect(resolveAgentBinary('codex', real)).toBe(real);
    const codex = checkAllProviders({ codex: real }).find((p) => p.provider === 'codex')!;
    expect(codex.found).toBe(true);
    expect(codex.resolvedPath).toBe(real);
  });

  it('a provider with nothing on PATH is not found', () => {
    const opencode = checkAllProviders().find((p) => p.provider === 'opencode')!;
    expect(opencode.found).toBe(false);
    expect(opencode.resolvedPath).toBeNull();
    // resolveAgentBinary falls back to the bare name so a fresh install works.
    expect(resolveAgentBinary('opencode')).toBe('opencode');
  });
});
