import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  collectEditedFiles,
  estimateSnapshot,
  captureTurnSnapshot,
} from './turnChanges';
import type { ToolCall } from '../types/claudeSession';

const tc = (name: string, input: any): ToolCall => ({
  id: `t-${name}-${Math.random()}`,
  name,
  input,
  status: 'complete',
  startedAt: 0,
});

describe('collectEditedFiles', () => {
  it('collects claude Edit/Write with line estimates', () => {
    const edited = collectEditedFiles([
      tc('Edit', { file_path: '/repo/src/a.ts', old_string: 'x\ny', new_string: 'x\ny\nz' }),
      tc('Write', { file_path: '/repo/b.md', content: 'one\ntwo\nthree' }),
      tc('Read', { file_path: '/repo/ignored.ts' }),
      tc('Bash', { command: 'rm -rf x' }),
    ]);
    expect([...edited.keys()]).toEqual(['/repo/src/a.ts', '/repo/b.md']);
    expect(edited.get('/repo/src/a.ts')).toEqual({ added: 3, removed: 2 });
    expect(edited.get('/repo/b.md')).toEqual({ added: 3, removed: 0 });
  });

  it('sums MultiEdit sub-edits and merges repeat edits to one file', () => {
    const edited = collectEditedFiles([
      tc('MultiEdit', {
        file_path: '/repo/a.ts',
        edits: [
          { old_string: 'a', new_string: 'b\nc' },
          { old_string: 'd\ne', new_string: 'f' },
        ],
      }),
      tc('Edit', { file_path: '/repo/a.ts', old_string: 'g', new_string: 'h' }),
    ]);
    expect(edited.size).toBe(1);
    expect(edited.get('/repo/a.ts')).toEqual({ added: 4, removed: 4 });
  });

  it('matches codex apply_patch (path only, no estimates)', () => {
    const edited = collectEditedFiles([tc('apply_patch', { path: 'src/main.rs' })]);
    expect(edited.get('src/main.rs')).toEqual({ added: 0, removed: 0 });
  });

  it('matches opencode/pi lowercase tools with filePath variants', () => {
    const edited = collectEditedFiles([
      tc('edit', { filePath: '/repo/x.go', old_string: 'a', new_string: 'b' }),
      tc('write', { path: '/repo/y.go', content: 'l1\nl2' }),
      tc('patch', { filePath: '/repo/z.go' }),
    ]);
    expect([...edited.keys()]).toEqual(['/repo/x.go', '/repo/y.go', '/repo/z.go']);
  });

  it('ignores calls without a usable path', () => {
    expect(collectEditedFiles([tc('Edit', { old_string: 'a', new_string: 'b' })]).size).toBe(0);
  });
});

describe('estimateSnapshot', () => {
  it('strips the cwd prefix and totals estimates', () => {
    const snap = estimateSnapshot(
      new Map([
        ['/repo/src/a.ts', { added: 3, removed: 1 }],
        ['/repo/b.md', { added: 2, removed: 0 }],
      ]),
      '/repo',
    );
    expect(snap.gitAvailable).toBe(false);
    expect(snap.files.map((f) => f.relPath)).toEqual(['src/a.ts', 'b.md']);
    expect(snap.totalAdded).toBe(5);
    expect(snap.totalRemoved).toBe(1);
  });
});

describe('captureTurnSnapshot', () => {
  beforeEach(() => {
    (window as any).electronAPI = {};
  });

  it('intersects tool paths with git status and uses numstat counts', async () => {
    (window as any).electronAPI = {
      gitStatus: vi.fn().mockResolvedValue({
        branch: 'main',
        files: [
          { path: 'src/a.ts', staged: ' ', unstaged: 'M' },
          { path: 'other/unrelated.ts', staged: ' ', unstaged: 'M' },
          { path: 'new.txt', staged: '?', unstaged: '?' },
        ],
      }),
      gitNumstat: vi
        .fn()
        // unstaged then staged (call order in captureTurnSnapshot)
        .mockResolvedValueOnce([
          { path: 'src/a.ts', added: 7, deleted: 2 },
          { path: 'other/unrelated.ts', added: 100, deleted: 100 },
        ])
        .mockResolvedValueOnce([{ path: 'src/a.ts', added: 3, deleted: 1 }]),
    };

    const snap = await captureTurnSnapshot(
      '/repo',
      new Map([
        ['/repo/src/a.ts', { added: 1, removed: 1 }],
        ['/repo/new.txt', { added: 4, removed: 0 }],
      ]),
    );
    expect(snap.gitAvailable).toBe(true);
    expect(snap.files).toHaveLength(2);
    const a = snap.files.find((f) => f.relPath === 'src/a.ts')!;
    // staged + unstaged summed; git numbers win over estimates
    expect(a).toMatchObject({ added: 10, removed: 3, code: 'M', untracked: false });
    const n = snap.files.find((f) => f.relPath === 'new.txt')!;
    // untracked files are absent from numstat — all-added from the estimate
    expect(n).toMatchObject({ added: 4, removed: 0, code: 'A', untracked: true });
    expect(snap.totalAdded).toBe(14);
    expect(snap.totalRemoved).toBe(3);
    // unrelated repo noise must not leak into the card
    expect(snap.files.some((f) => f.relPath.includes('unrelated'))).toBe(false);
  });

  it('falls back to estimates for a file committed during the turn', async () => {
    (window as any).electronAPI = {
      gitStatus: vi.fn().mockResolvedValue({ branch: 'main', files: [] }),
      gitNumstat: vi.fn().mockResolvedValue([]),
    };
    const snap = await captureTurnSnapshot('/repo', new Map([['/repo/a.ts', { added: 5, removed: 2 }]]));
    expect(snap.gitAvailable).toBe(true);
    expect(snap.files[0]).toMatchObject({ relPath: 'a.ts', added: 5, removed: 2 });
  });

  it('degrades to an estimate snapshot when git status rejects', async () => {
    (window as any).electronAPI = {
      gitStatus: vi.fn().mockRejectedValue(new Error('not a git repository')),
      gitNumstat: vi.fn(),
    };
    const snap = await captureTurnSnapshot('/tmp/nowhere', new Map([['/tmp/nowhere/a.ts', { added: 1, removed: 0 }]]));
    expect(snap.gitAvailable).toBe(false);
    expect(snap.files[0].relPath).toBe('a.ts');
  });

  it('treats binary numstat entries as binary (null counts)', async () => {
    (window as any).electronAPI = {
      gitStatus: vi.fn().mockResolvedValue({
        branch: 'main',
        files: [{ path: 'img.png', staged: ' ', unstaged: 'M' }],
      }),
      gitNumstat: vi
        .fn()
        .mockResolvedValueOnce([{ path: 'img.png', added: null, deleted: null }])
        .mockResolvedValueOnce([]),
    };
    const snap = await captureTurnSnapshot('/repo', new Map([['/repo/img.png', { added: 0, removed: 0 }]]));
    expect(snap.files[0]).toMatchObject({ added: null, removed: null });
  });
});
