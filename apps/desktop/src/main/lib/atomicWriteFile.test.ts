import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

// Mock `fs` by wrapping the REAL module so the happy paths hit a real tmpdir
// while a shared control object lets the failure tests inject errors into
// writeFileSync / renameSync / rmSync (ESM namespaces can't be spied directly).
const control = vi.hoisted(() => ({
  failWrite: false,
  failRename: false,
  failCleanup: false,
  writePaths: [] as string[],
}));

vi.mock('fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('fs')>();
  const errWith = (code: string, msg: string): NodeJS.ErrnoException => {
    const e = new Error(msg) as NodeJS.ErrnoException;
    e.code = code;
    return e;
  };
  const wrapped = {
    ...actual,
    writeFileSync: (p: fs.PathOrFileDescriptor, ...rest: unknown[]) => {
      control.writePaths.push(String(p));
      if (control.failWrite) throw errWith('ENOSPC', 'ENOSPC: no space left on device');
      return (actual.writeFileSync as unknown as (...a: unknown[]) => unknown)(p, ...rest);
    },
    renameSync: (from: fs.PathLike, to: fs.PathLike) => {
      if (control.failRename) throw errWith('EXDEV', 'EXDEV: cross-device rename');
      return actual.renameSync(from, to);
    },
    rmSync: (p: fs.PathLike, opts?: fs.RmOptions) => {
      if (control.failCleanup) throw new Error('cleanup blew up too');
      return actual.rmSync(p, opts);
    },
  };
  return { ...wrapped, default: wrapped };
});

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { atomicWriteFileSync } from './atomicWriteFile';

let dir = '';

beforeEach(() => {
  control.failWrite = false;
  control.failRename = false;
  control.failCleanup = false;
  control.writePaths = [];
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-atomic-'));
});

afterEach(() => {
  // Clear injected failures before cleanup so afterEach's own rmSync isn't
  // tripped by a test that left control.failCleanup on.
  control.failWrite = false;
  control.failRename = false;
  control.failCleanup = false;
  fs.rmSync(dir, { recursive: true, force: true });
});

/** Temp files left behind in `dir` — our temps are hidden dotfiles. */
function leftoverTemps(): string[] {
  return fs.readdirSync(dir).filter((f) => f.startsWith('.'));
}

describe('atomicWriteFileSync', () => {
  it('writes the final content to the target path', () => {
    const target = path.join(dir, 'config.yaml');
    atomicWriteFileSync(target, 'ui:\n  theme: dark\n');
    expect(fs.readFileSync(target, 'utf-8')).toBe('ui:\n  theme: dark\n');
  });

  it('leaves no temp files behind after a successful write', () => {
    const target = path.join(dir, 'config.yaml');
    atomicWriteFileSync(target, 'a: 1\n');
    expect(leftoverTemps()).toEqual([]);
    expect(fs.readdirSync(dir)).toEqual(['config.yaml']);
  });

  it('creates the parent directory if it does not exist', () => {
    const target = path.join(dir, 'nested', 'deeper', 'session.yaml');
    atomicWriteFileSync(target, 'name: x\n');
    expect(fs.readFileSync(target, 'utf-8')).toBe('name: x\n');
  });

  it('overwrites an existing file atomically', () => {
    const target = path.join(dir, 'config.yaml');
    fs.writeFileSync(target, 'old\n');
    atomicWriteFileSync(target, 'new\n');
    expect(fs.readFileSync(target, 'utf-8')).toBe('new\n');
    expect(leftoverTemps()).toEqual([]);
  });

  it('honours the file mode for secrets (0o600)', () => {
    const target = path.join(dir, 'tokens.json');
    atomicWriteFileSync(target, '[]\n', { mode: 0o600 });
    // Low 9 permission bits should be owner read/write only.
    expect(fs.statSync(target).mode & 0o777).toBe(0o600);
  });

  it('writes distinct temp names for rapid successive writes (no collision)', () => {
    // Two writes in the same tick must not reuse a temp name — the monotonic
    // counter guarantees uniqueness even within one millisecond.
    const target = path.join(dir, 'config.yaml');
    atomicWriteFileSync(target, '1');
    atomicWriteFileSync(target, '2');
    // Both temp paths were captured and they differ.
    expect(control.writePaths).toHaveLength(2);
    expect(new Set(control.writePaths).size).toBe(2);
    expect(fs.readFileSync(target, 'utf-8')).toBe('2');
  });

  describe('failure handling (leaves the original intact)', () => {
    it('a rename failure preserves the pre-existing target and cleans up the temp', () => {
      const target = path.join(dir, 'config.yaml');
      fs.writeFileSync(target, 'original\n');

      control.failRename = true;
      expect(() => atomicWriteFileSync(target, 'replacement\n')).toThrow(/EXDEV/);

      // Original file is untouched (never truncated) …
      expect(fs.readFileSync(target, 'utf-8')).toBe('original\n');
      // … and the temp file was cleaned up.
      expect(leftoverTemps()).toEqual([]);
    });

    it('a write failure never touches the target and cleans up', () => {
      const target = path.join(dir, 'config.yaml');
      fs.writeFileSync(target, 'original\n');

      control.failWrite = true;
      expect(() => atomicWriteFileSync(target, 'replacement\n')).toThrow(/ENOSPC/);

      expect(fs.readFileSync(target, 'utf-8')).toBe('original\n');
      expect(leftoverTemps()).toEqual([]);
    });

    it('swallows temp-cleanup errors and still rethrows the original error', () => {
      const target = path.join(dir, 'config.yaml');
      control.failRename = true;
      control.failCleanup = true;
      // The original rename error surfaces, not the cleanup error.
      expect(() => atomicWriteFileSync(target, 'x')).toThrow(/EXDEV/);
    });
  });
});
