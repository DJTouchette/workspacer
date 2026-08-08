import { describe, it, expect } from 'vitest';
import { isAsciiBlank, hasNonBlankText } from './asciiWhitespace';

// The desktop half of the caller-string trim seam. Its whole reason to exist is
// that `.trim()` and Go's strings.TrimSpace disagree on U+FEFF (BOM) and U+0085
// (NEL): `.trim()` strips the BOM, TrimSpace strips the NEL. If these two probes
// went back to `.trim()`, the BOM case below would flip (a lone-BOM title/path
// would read as blank here while the brain keeps it). TWIN: cmd/brain
// firstNonEmpty / fs.listDir, both trimming the shared asciiWhitespace set.
describe('asciiWhitespace blank predicate — cross-language parity', () => {
  const BOM = '\uFEFF';
  const NEL = '\u0085';

  it('treats empty and pure ASCII whitespace as blank', () => {
    for (const s of ['', ' ', '   ', '\t', '\n', '\v', '\f', '\r', ' \t\n\v\f\r ']) {
      expect(isAsciiBlank(s)).toBe(true);
      expect(hasNonBlankText(s)).toBe(false);
    }
  });

  it('does NOT treat a lone BOM as blank (the .trim() divergence)', () => {
    expect(isAsciiBlank(BOM)).toBe(false);
    expect(hasNonBlankText(BOM)).toBe(true);
  });

  it('does NOT treat a lone NEL as blank (the strings.TrimSpace divergence)', () => {
    expect(isAsciiBlank(NEL)).toBe(false);
    expect(hasNonBlankText(NEL)).toBe(true);
  });

  it('treats ordinary text as non-blank', () => {
    expect(isAsciiBlank('Notes')).toBe(false);
    expect(hasNonBlankText('Notes')).toBe(true);
    // Surrounding ASCII whitespace does not make a real title blank.
    expect(hasNonBlankText('  Notes  ')).toBe(true);
  });
});
