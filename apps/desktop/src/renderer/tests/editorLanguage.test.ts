/**
 * Regression test: language resolution must be case-insensitive on the file
 * extension. The CodeMirror language-data table is lowercase, so an uppercase
 * extension (Foo.TS, data.JSON) previously matched nothing and the editor fell
 * back to plain text with no highlighting.
 */
import { describe, it, expect } from 'vitest';
import { languageForPath } from '../src/panes/editor/language';

describe('languageForPath — case-insensitive extension match', () => {
  it('resolves uppercase extensions', () => {
    expect(languageForPath('Foo.TS')?.name).toBe('TypeScript');
    expect(languageForPath('data.JSON')?.name).toBe('JSON');
    expect(languageForPath('/a/b/Page.HTML')?.name).toBe('HTML');
  });

  it('still resolves lowercase extensions', () => {
    expect(languageForPath('app.tsx')?.name).toBe('TSX');
    expect(languageForPath('main.ts')?.name).toBe('TypeScript');
    expect(languageForPath('style.css')?.name).toBe('CSS');
  });

  it('returns undefined for an unknown or extensionless name', () => {
    expect(languageForPath('Makefilewithoutext')).toBeUndefined();
    expect(languageForPath('mystery.zzzz')).toBeUndefined();
  });
});
