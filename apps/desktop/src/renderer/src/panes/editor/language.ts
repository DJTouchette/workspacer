import { languages } from '@codemirror/language-data';
import type { LanguageDescription } from '@codemirror/language';

/**
 * Resolve the CodeMirror language description for a file path by its extension.
 *
 * `@codemirror/language-data` stores every language's `extensions` array in
 * lowercase, so the extension must be lowercased before matching — otherwise a
 * file like `Foo.TS` or `data.JSON` fails the lookup and renders as plain text
 * with no syntax highlighting. (The file-tree icon path already lowercases for
 * the same reason.)
 */
export function languageForPath(filePath: string): LanguageDescription | undefined {
  const ext = filePath.split(/[\\/]/).pop()?.split('.').pop()?.toLowerCase();
  if (!ext) return undefined;
  return languages.find((l) => l.extensions.includes(ext));
}
