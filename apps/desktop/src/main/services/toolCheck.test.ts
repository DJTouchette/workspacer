import { describe, it, expect, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { resolveOnPath, toolsStatus, TOOL_REGISTRY } from './toolCheck';

const savedPath = process.env.PATH;
afterEach(() => {
  process.env.PATH = savedPath;
});

describe('toolCheck', () => {
  it('resolveOnPath finds a binary in a PATH dir and misses an absent one', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-tools-'));
    const bin = path.join(dir, process.platform === 'win32' ? 'fakegit.exe' : 'fakegit');
    fs.writeFileSync(bin, '#!/bin/sh\n', { mode: 0o755 });
    process.env.PATH = dir;
    expect(resolveOnPath('fakegit')).toBe(bin);
    expect(resolveOnPath('definitely-not-a-tool')).toBeUndefined();
  });

  it('caches until forced — a force re-scan sees PATH changes', () => {
    process.env.PATH = '';
    const before = toolsStatus(true).find((t) => t.id === 'git')!;
    expect(before.available).toBe(false);

    // git appears on PATH; the cached answer stays until force.
    process.env.PATH = savedPath ?? '';
    expect(toolsStatus().find((t) => t.id === 'git')!.available).toBe(false);
    const after = toolsStatus(true).find((t) => t.id === 'git')!;
    // (asserts the re-scan ran; whether git exists depends on the host)
    expect(after.available).toBe(!!resolveOnPath('git'));
  });

  it('every registry entry documents its features and an install hint', () => {
    for (const spec of TOOL_REGISTRY) {
      expect(spec.features.length, spec.id).toBeGreaterThan(0);
      expect(spec.install.length, spec.id).toBeGreaterThan(0);
    }
  });
});
