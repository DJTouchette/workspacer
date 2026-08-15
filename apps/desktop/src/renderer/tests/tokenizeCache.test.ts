/**
 * `tokenize` memoizes its RESULT, not just the grammar.
 *
 * The grammar was already cached; the tokenization was not. So the same snippet
 * went through the TextMate engine again on every render that asked for it —
 * and `codeToTokensBase` is synchronous and blocking (a dense 300-line block
 * measures ~220ms), so those passes pile up on the main thread rather than
 * coalescing. A streaming code fence hit this hardest, since each flush is a
 * slightly longer string whose prefix was just tokenized.
 *
 * Nothing else in the suite would notice if the cache were removed: the output
 * would be identical and only the speed would change.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

const codeToTokensBase = vi.fn((code: string) =>
  code.split('\n').map((l) => [{ content: l, color: '#fff' }]),
);

vi.mock('@shikijs/core', () => ({
  createHighlighterCore: vi.fn(async () => ({
    codeToTokensBase,
    loadLanguage: vi.fn(async () => {}),
  })),
}));
vi.mock('@shikijs/engine-javascript', () => ({
  createJavaScriptRegexEngine: vi.fn(() => ({})),
}));
vi.mock('@shikijs/langs/typescript', () => ({ default: [{ name: 'typescript' }] }));
vi.mock('@shikijs/langs/cpp', () => ({ default: [{ name: 'cpp' }] }));

import { tokenize, clearTokenCache } from '../src/lib/diff/highlight';

beforeEach(() => {
  codeToTokensBase.mockClear();
  clearTokenCache();
});

describe('tokenize', () => {
  it('tokenizes identical input only once', async () => {
    const code = 'const a = 1\nconst b = 2';
    const first = await tokenize(code, 'typescript');
    const second = await tokenize(code, 'typescript');

    expect(codeToTokensBase).toHaveBeenCalledTimes(1);
    // The cached value is returned, not recomputed.
    expect(second).toEqual(first);
  });

  it('does not confuse two different snippets', async () => {
    await tokenize('const a = 1', 'typescript');
    await tokenize('const b = 2', 'typescript');
    expect(codeToTokensBase).toHaveBeenCalledTimes(2);
  });

  it('re-tokenizes after the cache is cleared on session switch', async () => {
    const code = 'const a = 1';
    await tokenize(code, 'typescript');
    clearTokenCache();
    await tokenize(code, 'typescript');
    expect(codeToTokensBase).toHaveBeenCalledTimes(2);
  });

  it('is bounded — a long streaming session cannot grow it without limit', async () => {
    // 400 distinct snippets against a 300-entry cap. The oldest must have been
    // evicted, or a session that streams for hours accumulates every prefix of
    // every code block it ever rendered.
    for (let i = 0; i < 400; i++) await tokenize(`line ${i}`, 'typescript');
    expect(codeToTokensBase).toHaveBeenCalledTimes(400);

    codeToTokensBase.mockClear();
    await tokenize('line 399', 'typescript'); // most recent — still cached
    expect(codeToTokensBase).not.toHaveBeenCalled();

    await tokenize('line 0', 'typescript'); // oldest — evicted
    expect(codeToTokensBase).toHaveBeenCalledTimes(1);
  });
});
