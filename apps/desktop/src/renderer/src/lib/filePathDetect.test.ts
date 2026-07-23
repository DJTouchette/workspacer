/**
 * Detection contract for file paths in LLM text (lib/filePathDetect).
 * Precision is the whole game: these tests pin what must NOT linkify (domains,
 * URLs, dotted identifiers, flags, versions) as firmly as what must.
 */
import { describe, it, expect } from 'vitest';
import { detectFilePath, linkifyText, type DetectedPath } from './filePathDetect';

describe('detectFilePath (code spans)', () => {
  it('accepts absolute, dot-relative, and multi-segment paths', () => {
    expect(detectFilePath('/home/me/app/src/main.ts')?.path).toBe('/home/me/app/src/main.ts');
    expect(detectFilePath('./scripts/build.sh')?.path).toBe('./scripts/build.sh');
    expect(detectFilePath('../lib/util.py')?.path).toBe('../lib/util.py');
    expect(detectFilePath('src/main/services/agentNotifier.ts')?.path).toBe(
      'src/main/services/agentNotifier.ts',
    );
    expect(detectFilePath('C:\\Users\\me\\app.rs')?.path).toBe('C:\\Users\\me\\app.rs');
  });

  it('accepts single-segment filenames only for code-ish extensions', () => {
    expect(detectFilePath('package.json')?.path).toBe('package.json');
    expect(detectFilePath('config.yaml')?.path).toBe('config.yaml');
    expect(detectFilePath('Makefile')?.path).toBe('Makefile');
    expect(detectFilePath('example.com')).toBeNull(); // TLD, not extension
    expect(detectFilePath('workspacer.ai')).toBeNull();
    expect(detectFilePath('notifications.post')).toBeNull(); // dotted identifier
  });

  it('parses and strips file:line suffixes but keeps them in display', () => {
    const hit = detectFilePath('src/app.ts:123') as DetectedPath;
    expect(hit.path).toBe('src/app.ts');
    expect(hit.line).toBe(123);
    expect(hit.display).toBe('src/app.ts:123');
    expect(detectFilePath('src/app.ts:12:5')?.path).toBe('src/app.ts');
  });

  it('rejects URLs, flags, globs, commands, dirs, and versions', () => {
    expect(detectFilePath('https://a.com/b.ts')).toBeNull();
    expect(detectFilePath('--plugins-dir')).toBeNull();
    expect(detectFilePath('src/**/*.ts')).toBeNull();
    expect(detectFilePath('cat src/foo.ts')).toBeNull(); // whitespace = command
    expect(detectFilePath('services/hub/internal/')).toBeNull(); // trailing slash
    expect(detectFilePath('services/hub/internal')).toBeNull(); // no file-like basename
    expect(detectFilePath('v1.2')).toBeNull();
    expect(detectFilePath('github.com/owner/repo.git')).toBeNull(); // domain first segment
    expect(detectFilePath('key:value/pair.ts')).toBeNull(); // interior colon
  });
});

describe('linkifyText (prose)', () => {
  const paths = (parts: Array<string | DetectedPath>) =>
    parts.filter((p): p is DetectedPath => typeof p !== 'string').map((p) => p.path);
  const rejoin = (parts: Array<string | DetectedPath>) =>
    parts.map((p) => (typeof p === 'string' ? p : p.display)).join('');

  it('links separator-bearing paths and preserves surrounding text exactly', () => {
    const text = 'The bug is in src/main/index.ts:42 (see also ./docs/notes.md).';
    const parts = linkifyText(text);
    expect(paths(parts)).toEqual(['src/main/index.ts', './docs/notes.md']);
    expect(rejoin(parts)).toBe(text);
  });

  it('never links bare filenames or dotted identifiers in prose', () => {
    expect(paths(linkifyText('Update package.json and call notifications.post.'))).toEqual([]);
  });

  it('trims trailing clause punctuation off the link', () => {
    const parts = linkifyText('Fixed in apps/desktop/src/main/preload.ts, then shipped.');
    expect(paths(parts)).toEqual(['apps/desktop/src/main/preload.ts']);
    expect(rejoin(parts)).toBe('Fixed in apps/desktop/src/main/preload.ts, then shipped.');
  });

  it('keeps a :line suffix while trimming punctuation after it', () => {
    const parts = linkifyText('See services/hub/cmd/hub/main.go:392.');
    const hit = parts.find((p): p is DetectedPath => typeof p !== 'string');
    expect(hit?.path).toBe('services/hub/cmd/hub/main.go');
    expect(hit?.line).toBe(392);
    expect(rejoin(parts)).toBe('See services/hub/cmd/hub/main.go:392.');
  });

  it('ignores URLs wholesale', () => {
    expect(
      paths(linkifyText('Docs at https://example.com/guide/setup.md and http://a.io/x.ts')),
    ).toEqual([]);
  });

  it('handles parenthesized paths', () => {
    const text = 'The handler (src/main/ipc.ts) owns it.';
    const parts = linkifyText(text);
    expect(paths(parts)).toEqual(['src/main/ipc.ts']);
    expect(rejoin(parts)).toBe(text);
  });

  it('returns plain text untouched when nothing matches', () => {
    expect(linkifyText('no paths here at all')).toEqual(['no paths here at all']);
  });
});
