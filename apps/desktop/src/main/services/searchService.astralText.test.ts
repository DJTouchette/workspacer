/**
 * `search.project`'s `text` field, and its `maxResults` floor — the two places
 * the desktop copy and the brain answered the same request differently.
 *
 * search.project is registered by BOTH providers (hubCapabilities.ts and
 * cmd/brain/handlers.go), and under the default DELEGATE_CATALOG_TO_BRAIN the
 * brain is the one that answers — so a divergence here is not a desktop bug or a
 * brain bug, it is one method with two behaviours.
 *
 *   CLIP        `String.slice` counts UTF-16 CODE UNITS; Go's clip counts RUNES.
 *               A matching line of astral characters came back at 300 code
 *               points from the brain and 153 from here, and an ODD boundary
 *               left a LONE LEAD SURROGATE (0xD83D) that JSON.stringify emits on
 *               the wire as a bare `\ud83d` — Go's json.Unmarshal turns it into
 *               U+FFFD and a strict JSON reader rejects the frame outright.
 *   TRIM        JS `.trim()` and Go's `strings.TrimSpace` disagree on U+FEFF and
 *               U+0085, in opposite directions, so a line beginning with a BOM
 *               came back different from each.
 *   maxResults  `opts.maxResults ?? DEFAULT` only replaces null/undefined, so
 *               `maxResults: 0` was a literal cap of ZERO here (an empty result
 *               list, spuriously flagged truncated) and "unset, use 500" in the
 *               brain (`if maxResults <= 0`). A web client that computes its cap
 *               and lands on 0 got a full list headless and nothing at all under
 *               the shipping layout.
 *
 * Nothing pinned any of it: the only other test on this file
 * (searchService.truncate.test.ts) asserts the `truncated` flag and never
 * inspects `text`.
 *
 * TWIN: TestParseRipgrepJSONClipsCodePointsAndTrimsAsciiOnly and
 * TestEffectiveMaxResultsTreatsNonPositiveAsUnset in
 * services/hub/cmd/brain/search_test.go, with these same vectors.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { searchProject } from './searchService';

const EMOJI = '\u{1F600}';
/** The cap searchService applies when the caller names none. Mirrored from the
 *  module's own MAX_TEXT_LEN / searchMaxTextLen — both are 300. */
const MAX_TEXT_LEN = 300;

let dir: string;
beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-search-astral-'));
});
afterEach(() => {
  fs.rmSync(dir, { recursive: true, force: true });
});

describe('searchProject — text is clipped in code points and trimmed like the brain', () => {
  it('keeps 300 CODE POINTS and never splits an astral character', async () => {
    // Odd boundary: one ASCII byte then astral characters, so a UTF-16 counter
    // stops mid-surrogate-pair. Even boundary: all astral, where a UTF-16
    // counter produces valid-but-halved text — invisible without counting.
    fs.writeFileSync(path.join(dir, 'odd.txt'), 'a' + EMOJI.repeat(400) + '\n');
    fs.writeFileSync(path.join(dir, 'even.txt'), EMOJI.repeat(400) + '\n');

    const r = await searchProject({ query: EMOJI, cwd: dir, caseSensitive: true });
    const texts = new Map(r.results.map((f) => [path.basename(f.file), f.matches[0]?.text ?? '']));
    for (const name of ['odd.txt', 'even.txt']) {
      const text = texts.get(name) ?? '';
      expect([...text].length, `${name}: code-point length (a UTF-16 slice keeps about half)`).toBe(
        MAX_TEXT_LEN,
      );
      // A lone surrogate is what a UTF-16 slice leaves at an odd boundary, and
      // it is what JSON.stringify emits as a bare \ud83d on the bus.
      expect(/[\uD800-\uDFFF]/.test(text.replace(/[\uD800-\uDBFF][\uDC00-\uDFFF]/g, ''))).toBe(
        false,
      );
      expect(text.endsWith(EMOJI), `${name}: the final character was split`).toBe(true);
    }
  });

  it('trims the ASCII whitespace set only, so U+FEFF and U+0085 survive', async () => {
    // U+FEFF: whitespace to JS `.trim()`, not to Go's TrimSpace.
    // U+0085: whitespace to Go's TrimSpace, not to JS `.trim()`.
    const line = '\ufeffNEEDLE\u0085';
    // NOT the first line of the file: ripgrep strips a leading BOM as an
    // encoding marker before the line ever reaches us, so a file that starts
    // with one would test the trim against a string that no longer has it.
    fs.writeFileSync(path.join(dir, 'bom.txt'), 'header\n' + line + '\n');

    const r = await searchProject({ query: 'NEEDLE', cwd: dir, caseSensitive: true });
    expect(r.results[0]?.matches[0]?.text).toBe(line);
  });

  it('treats a non-positive maxResults as UNSET, not as a cap of zero', async () => {
    fs.writeFileSync(path.join(dir, 'a.txt'), 'NEEDLE\nNEEDLE\nNEEDLE\n');
    for (const maxResults of [0, -1]) {
      const r = await searchProject({ query: 'NEEDLE', cwd: dir, caseSensitive: true, maxResults });
      const total = r.results.reduce((n, f) => n + f.matches.length, 0);
      expect(
        total,
        `maxResults: ${maxResults} must mean "use the default", as it does in the brain`,
      ).toBe(3);
      expect(
        r.truncated,
        `maxResults: ${maxResults} dropped nothing, so nothing was truncated`,
      ).toBe(false);
    }
    // The floor: a real cap still caps.
    const capped = await searchProject({
      query: 'NEEDLE',
      cwd: dir,
      caseSensitive: true,
      maxResults: 2,
    });
    expect(capped.results.reduce((n, f) => n + f.matches.length, 0)).toBe(2);
    expect(capped.truncated).toBe(true);
  });
});
