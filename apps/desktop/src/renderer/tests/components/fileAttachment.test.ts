import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  extractFilePaths,
  classifyFile,
  buildPromptPrefix,
  mergeAttachments,
} from '../../src/components/claude/fileAttachment';

/**
 * Drag-and-drop / paste attachment resolution.
 *
 * This is where dropping a file into chat silently stopped working: Electron 32
 * removed the `File.path` augmentation but left its *type declaration* in
 * electron.d.ts, so `f.path` kept compiling and started returning undefined at
 * runtime — every drop resolved to zero paths, the overlay flashed, nothing
 * attached. The path now comes from webUtils via the preload bridge, so these
 * tests pin the bridge call and the fallbacks around it.
 */

const getPathForFile = vi.fn<(f: File) => string>();

beforeEach(() => {
  getPathForFile.mockReset();
  (window as unknown as { electronAPI: unknown }).electronAPI = { getPathForFile };
});

/** Minimal DataTransfer stand-in — jsdom has no constructible one. */
function transfer(opts: { files?: File[]; data?: Record<string, string> } = {}): DataTransfer {
  return {
    files: opts.files ?? [],
    getData: (type: string) => opts.data?.[type] ?? '',
  } as unknown as DataTransfer;
}

const file = (name: string) => new File(['x'], name);

describe('extractFilePaths', () => {
  it('resolves dropped files through the preload bridge', () => {
    const [a, b] = [file('a.png'), file('b.pdf')];
    getPathForFile.mockImplementation((f) => `/repo/${f.name}`);

    expect(extractFilePaths(transfer({ files: [a, b] }))).toEqual(['/repo/a.png', '/repo/b.pdf']);
    expect(getPathForFile).toHaveBeenCalledTimes(2);
  });

  it('skips a File with no path on disk (pasted screenshot → empty string)', () => {
    getPathForFile.mockReturnValue('');
    expect(extractFilePaths(transfer({ files: [file('image.png')] }))).toEqual([]);
  });

  it('still honours the legacy File.path when the bridge is unavailable', () => {
    (window as unknown as { electronAPI: unknown }).electronAPI = {};
    const f = Object.assign(file('legacy.png'), { path: '/repo/legacy.png' });
    expect(extractFilePaths(transfer({ files: [f] }))).toEqual(['/repo/legacy.png']);
  });

  it('falls back to text/uri-list when no file resolved a path', () => {
    getPathForFile.mockReturnValue('');
    const uriList = [
      '# a uri-list comment',
      'file:///repo/my%20shot.png',
      'https://example.com/not-a-file.png',
      'file:///repo/two.pdf',
    ].join('\r\n');

    expect(extractFilePaths(transfer({ data: { 'text/uri-list': uriList } }))).toEqual([
      '/repo/my shot.png',
      '/repo/two.pdf',
    ]);
  });

  it('prefers real files over the uri-list rather than attaching both', () => {
    getPathForFile.mockReturnValue('/repo/a.png');
    const dt = transfer({
      files: [file('a.png')],
      data: { 'text/uri-list': 'file:///repo/a.png' },
    });
    expect(extractFilePaths(dt)).toEqual(['/repo/a.png']);
  });

  it('strips the leading slash from a Windows drive path', () => {
    getPathForFile.mockReturnValue('');
    const dt = transfer({ data: { 'text/uri-list': 'file:///C:/Users/me/shot.png' } });
    // new URL().pathname would give "/C:/Users/me/shot.png", which is not a path.
    expect(extractFilePaths(dt)).toEqual(['C:/Users/me/shot.png']);
  });

  it('rebuilds a UNC path from a file:// URI with a host', () => {
    getPathForFile.mockReturnValue('');
    const dt = transfer({ data: { 'text/uri-list': 'file://nas/share/report.pdf' } });
    expect(extractFilePaths(dt)).toEqual(['\\\\nas\\share\\report.pdf']);
  });

  it('returns nothing for a plain-text drag', () => {
    expect(extractFilePaths(transfer({ data: { 'text/plain': 'hello' } }))).toEqual([]);
  });
});

describe('classifyFile / buildPromptPrefix', () => {
  it('labels by extension, case-insensitively', () => {
    expect(classifyFile('/repo/Shot.PNG').label).toBe('Image');
    expect(classifyFile('/repo/spec.pdf').label).toBe('PDF');
    expect(classifyFile('/repo/notes.md').label).toBe('File');
  });

  it('keeps the basename for display and the full path for the prompt', () => {
    const f = classifyFile('/repo/deep/dir/shot.png');
    expect(f.name).toBe('shot.png');
    expect(buildPromptPrefix([f])).toBe('[Image: /repo/deep/dir/shot.png] ');
  });
});

describe('mergeAttachments', () => {
  const f = (path: string) => classifyFile(path);

  it('drops a path that is already attached', () => {
    const existing = [f('/repo/a.png')];
    expect(mergeAttachments(existing, [f('/repo/a.png')])).toBe(existing); // same array: no churn
  });

  it('appends only the new paths', () => {
    const merged = mergeAttachments([f('/repo/a.png')], [f('/repo/a.png'), f('/repo/b.pdf')]);
    expect(merged.map((x) => x.path)).toEqual(['/repo/a.png', '/repo/b.pdf']);
  });

  it('dedupes within a single batch', () => {
    const merged = mergeAttachments([], [f('/repo/a.png'), f('/repo/a.png')]);
    expect(merged).toHaveLength(1);
  });
});
