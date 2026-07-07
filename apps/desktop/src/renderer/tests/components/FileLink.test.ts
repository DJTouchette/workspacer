import { describe, it, expect } from 'vitest';
import {
  isAbsolutePath,
  resolveWithCwd,
  isMarkdownPath,
  isHtmlPath,
  openFileDefault,
} from '../../src/components/claude/FileLink';
import { EDITOR_OPEN_FILE_EVENT, type EditorOpenTarget } from '../../src/lib/editorBus';
import { MARKDOWN_PREVIEW_EVENT, type MarkdownPreviewTarget } from '../../src/lib/previewBus';

/**
 * FileLink's path helpers are the single source of truth for "what does
 * clicking a file path do" across the chat's tool-call UI. Pure-function
 * coverage here: absolute-path detection across platforms, relative→absolute
 * resolution against the session cwd, extension sniffing, and the default
 * left-click dispatch (markdown → preview bus, everything else → editor bus).
 */

/** Collect one dispatch of a window CustomEvent while `fn` runs. */
function captureEvent<T>(eventName: string, fn: () => void): T[] {
  const seen: T[] = [];
  const listener = (e: Event) => seen.push((e as CustomEvent<T>).detail);
  window.addEventListener(eventName, listener);
  try {
    fn();
  } finally {
    window.removeEventListener(eventName, listener);
  }
  return seen;
}

describe('isAbsolutePath', () => {
  it('detects POSIX absolute paths', () => {
    expect(isAbsolutePath('/home/user/file.ts')).toBe(true);
    expect(isAbsolutePath('/')).toBe(true);
  });

  it('detects Windows drive paths with either slash', () => {
    expect(isAbsolutePath('C:\\Users\\me\\file.ts')).toBe(true);
    expect(isAbsolutePath('c:/Users/me/file.ts')).toBe(true);
  });

  it('detects UNC paths', () => {
    expect(isAbsolutePath('\\\\server\\share\\file.ts')).toBe(true);
  });

  it('rejects relative paths', () => {
    expect(isAbsolutePath('src/App.tsx')).toBe(false);
    expect(isAbsolutePath('./src/App.tsx')).toBe(false);
    expect(isAbsolutePath('../up/one.ts')).toBe(false);
    expect(isAbsolutePath('')).toBe(false);
    // A lone drive letter without a separator is not a drive path.
    expect(isAbsolutePath('C:file.ts')).toBe(false);
  });
});

describe('resolveWithCwd', () => {
  it('passes absolute paths through untouched, even with a cwd', () => {
    expect(resolveWithCwd('/abs/file.ts', '/repo')).toBe('/abs/file.ts');
    expect(resolveWithCwd('C:\\abs\\file.ts', '/repo')).toBe('C:\\abs\\file.ts');
  });

  it('resolves relative paths against the cwd', () => {
    expect(resolveWithCwd('src/App.tsx', '/repo')).toBe('/repo/src/App.tsx');
  });

  it('does not double the separator when the cwd has trailing slashes', () => {
    expect(resolveWithCwd('src/App.tsx', '/repo/')).toBe('/repo/src/App.tsx');
    expect(resolveWithCwd('src/App.tsx', '/repo///')).toBe('/repo/src/App.tsx');
    expect(resolveWithCwd('src\\App.tsx', 'C:\\repo\\')).toBe('C:\\repo/src\\App.tsx');
  });

  it('returns a relative path as-is when there is no cwd to resolve against', () => {
    expect(resolveWithCwd('src/App.tsx')).toBe('src/App.tsx');
    expect(resolveWithCwd('src/App.tsx', undefined)).toBe('src/App.tsx');
  });
});

describe('extension detection', () => {
  it('matches markdown extensions case-insensitively', () => {
    expect(isMarkdownPath('/notes/README.md')).toBe(true);
    expect(isMarkdownPath('/notes/README.MD')).toBe(true);
    expect(isMarkdownPath('/notes/guide.markdown')).toBe(true);
    expect(isMarkdownPath('/notes/guide.Markdown')).toBe(true);
  });

  it('does not treat near-misses as markdown', () => {
    expect(isMarkdownPath('/notes/page.mdx')).toBe(false);
    expect(isMarkdownPath('/notes/md')).toBe(false);
    expect(isMarkdownPath('/notes/file.ts')).toBe(false);
  });

  it('matches html extensions case-insensitively', () => {
    expect(isHtmlPath('/site/index.html')).toBe(true);
    expect(isHtmlPath('/site/index.HTML')).toBe(true);
    expect(isHtmlPath('/site/index.htm')).toBe(true);
  });

  it('does not treat near-misses as html', () => {
    expect(isHtmlPath('/site/index.xhtml')).toBe(false);
    expect(isHtmlPath('/site/index.html.bak')).toBe(false);
    expect(isHtmlPath('/site/main.ts')).toBe(false);
  });
});

describe('openFileDefault', () => {
  it('routes markdown files to the preview bus with the cwd-resolved path', () => {
    const previews = captureEvent<MarkdownPreviewTarget>(MARKDOWN_PREVIEW_EVENT, () =>
      openFileDefault('docs/notes.md', '/repo'),
    );
    expect(previews).toEqual([{ path: '/repo/docs/notes.md', cwd: '/repo' }]);
  });

  it('does not touch the editor bus for markdown files', () => {
    const edits = captureEvent<EditorOpenTarget>(EDITOR_OPEN_FILE_EVENT, () =>
      openFileDefault('/repo/docs/notes.md', '/repo'),
    );
    expect(edits).toEqual([]);
  });

  it('routes everything else to the editor bus with the cwd-resolved path', () => {
    const edits = captureEvent<EditorOpenTarget>(EDITOR_OPEN_FILE_EVENT, () =>
      openFileDefault('src/App.tsx', '/repo'),
    );
    expect(edits).toEqual([{ path: '/repo/src/App.tsx', cwd: '/repo' }]);
  });

  it('does not touch the preview bus for non-markdown files', () => {
    const previews = captureEvent<MarkdownPreviewTarget>(MARKDOWN_PREVIEW_EVENT, () =>
      openFileDefault('/abs/main.rs'),
    );
    expect(previews).toEqual([]);
  });

  it('routes an already-absolute path without a cwd to the editor untouched', () => {
    const edits = captureEvent<EditorOpenTarget>(EDITOR_OPEN_FILE_EVENT, () =>
      openFileDefault('/abs/main.rs'),
    );
    expect(edits).toEqual([{ path: '/abs/main.rs', cwd: undefined }]);
  });
});
