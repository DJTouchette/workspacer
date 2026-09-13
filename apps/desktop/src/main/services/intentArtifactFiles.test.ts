import fs from 'node:fs';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { afterEach, expect, it, vi } from 'vitest';
import { openIntentArtifactDirectory, openIntentArtifactFile } from './intentArtifactFiles';
const roots: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});
const directory = () => {
  const root = fs.mkdtempSync(path.join(tmpdir(), 'intent-files-'));
  roots.push(root);
  return root;
};
const leaf = '11111111-1111-1111-1111-111111111111.bin';
it('rejects symlink directories and leaf files, including symlinked missing ancestors', () => {
  const root = directory();
  const outside = directory();
  fs.symlinkSync(outside, path.join(root, 'linked'));
  expect(() => openIntentArtifactFile(path.join(root, 'linked/new'), leaf, true)).toThrow();
  expect(fs.readdirSync(outside)).toEqual([]);
  fs.writeFileSync(path.join(outside, 'secret'), 'secret');
  fs.symlinkSync(path.join(outside, 'secret'), path.join(root, leaf));
  expect(() => openIntentArtifactFile(root, leaf)).toThrow();
  expect(() => openIntentArtifactFile(root, '../secret.bin')).toThrow('filename');
});
it('pins directory handles across a malicious parent-path swap before the file open', () => {
  const root = directory();
  const outside = directory();
  const original = path.join(root, 'artifacts');
  const retained = path.join(root, 'retained');
  fs.mkdirSync(original);
  const open = fs.openSync.bind(fs);
  let swapped = false;
  vi.spyOn(fs, 'openSync').mockImplementation(((
    filename: fs.PathLike,
    flags: string | number,
    mode?: fs.Mode,
  ) => {
    if (!swapped && String(filename).endsWith('/' + leaf)) {
      swapped = true;
      fs.renameSync(original, retained);
      fs.symlinkSync(outside, original);
    }
    return open(filename, flags, mode);
  }) as typeof fs.openSync);
  const file = openIntentArtifactFile(original, leaf, true);
  try {
    fs.writeFileSync(file.fd, 'host-owned bytes');
  } finally {
    file.close();
  }
  expect(swapped).toBe(true);
  expect(fs.readdirSync(outside)).toEqual([]);
  expect(fs.readFileSync(path.join(retained, leaf), 'utf8')).toBe('host-owned bytes');
});
it('creates descendants beneath a pinned parent and prevents path traversal', () => {
  const root = directory();
  const handle = openIntentArtifactDirectory(path.join(root, 'nested/artifacts'), true);
  try {
    expect(() => handle.leafPath('../escape')).toThrow();
    expect(() => handle.leafPath('x/y')).toThrow();
    expect(() => handle.leafPath('x\\y')).toThrow();
    expect(handle.leafPath('record.md')).toMatch(/^\/proc\/self\/fd\/\d+\/record.md$/);
  } finally {
    handle.close();
  }
});
